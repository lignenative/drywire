//! Account vaults (section 15): the code a person types or keeps, the id
//! and key it derives, and the sealed bytes an instance stores without
//! being able to open them (ADR 0051).
//!
//! What goes inside a vault is the caller's business (JSON, section
//! 15.2): this module seals and opens bytes.

use aes_gcm::Aes256Gcm;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use hkdf::Hkdf;
use sha2::Sha256;

/// Size of a code's secret.
pub const SECRET_SIZE: usize = 16;
/// Size of a vault's id.
pub const ID_SIZE: usize = 32;
/// Characters of a code, hyphens left out.
pub const CODE_LENGTH: usize = 26;
/// The only version of the sealed bytes.
pub const VERSION: u8 = 1;
const NONCE_SIZE: usize = 12;

const ID_INFO: &[u8] = b"drywire/vault/id/v1";
const KEY_INFO: &[u8] = b"drywire/vault/key/v1";
const AAD_PREFIX: &[u8] = b"drywire/vault/v1";
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Why a code or a vault was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultError {
    /// The text is not a code: a wrong character, length or spare bits.
    BadCode,
    /// The sealed bytes are of a version this client does not know.
    UnsupportedVersion,
    /// The sealed bytes do not open with this code: a wrong code, or
    /// damaged bytes.
    WrongCode,
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            VaultError::BadCode => "this is not a Drywire code",
            VaultError::UnsupportedVersion => "this vault needs a newer version of Drywire",
            VaultError::WrongCode => "this code does not open this vault",
        })
    }
}

impl std::error::Error for VaultError {}

/// A code: the secret behind one vault.
#[derive(Clone, PartialEq, Eq)]
pub struct VaultCode([u8; SECRET_SIZE]);

impl std::fmt::Debug for VaultCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A code is a secret: never in a log by accident.
        f.write_str("VaultCode(…)")
    }
}

impl VaultCode {
    /// The code of these 16 random bytes.
    pub fn from_secret(secret: [u8; SECRET_SIZE]) -> VaultCode {
        VaultCode(secret)
    }

    /// The secret, to keep on the device that made the code.
    pub fn secret(&self) -> [u8; SECRET_SIZE] {
        self.0
    }

    /// Reads a code as a person typed it (section 15.1).
    pub fn parse(text: &str) -> Result<VaultCode, VaultError> {
        let mut bits: u128 = 0;
        let mut count = 0;
        let mut last = 0u8;
        for c in text.chars() {
            let value = match c.to_ascii_uppercase() {
                '-' | ' ' => continue,
                'O' => 0,
                'I' | 'L' => 1,
                c if c.is_ascii() => ALPHABET
                    .iter()
                    .position(|&a| a == c as u8)
                    .ok_or(VaultError::BadCode)? as u8,
                _ => return Err(VaultError::BadCode),
            };
            count += 1;
            if count > CODE_LENGTH {
                return Err(VaultError::BadCode);
            }
            if count < CODE_LENGTH {
                bits = (bits << 5) | value as u128;
            } else {
                last = value;
            }
        }
        if count != CODE_LENGTH || last & 0b11 != 0 {
            return Err(VaultError::BadCode);
        }
        // 25 characters gave 125 bits; the last gives 3 more.
        let secret = (bits << 3) | (last >> 2) as u128;
        Ok(VaultCode(secret.to_be_bytes()))
    }

    /// The code as shown to a person, in groups of four.
    pub fn to_text(&self) -> String {
        let bits = u128::from_be_bytes(self.0);
        let mut out = String::with_capacity(CODE_LENGTH + 6);
        for i in 0..CODE_LENGTH {
            if i > 0 && i % 4 == 0 {
                out.push('-');
            }
            let value = if i < CODE_LENGTH - 1 {
                (bits >> (123 - 5 * i)) & 0x1f
            } else {
                (bits & 0b111) << 2
            };
            out.push(ALPHABET[value as usize] as char);
        }
        out
    }

    /// The id an instance keeps the vault under.
    pub fn id(&self) -> [u8; ID_SIZE] {
        derive(&self.0, ID_INFO)
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new(&derive(&self.0, KEY_INFO).into())
    }

    fn aad(&self) -> Vec<u8> {
        [AAD_PREFIX, &self.id()].concat()
    }

    /// Seals a vault's plaintext, with a nonce the caller draws at random.
    pub fn seal(&self, plaintext: &[u8], nonce: [u8; NONCE_SIZE]) -> Vec<u8> {
        let aad = self.aad();
        let ciphertext = self
            .cipher()
            .encrypt(
                &nonce.into(),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .expect("AES-GCM seals any length a vault has");
        [&[VERSION][..], &nonce, &ciphertext].concat()
    }

    /// Opens sealed bytes made with this code.
    pub fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, VaultError> {
        let Some((&version, rest)) = sealed.split_first() else {
            return Err(VaultError::WrongCode);
        };
        if version != VERSION {
            return Err(VaultError::UnsupportedVersion);
        }
        if rest.len() < NONCE_SIZE {
            return Err(VaultError::WrongCode);
        }
        let (nonce, ciphertext) = rest.split_at(NONCE_SIZE);
        let nonce: [u8; NONCE_SIZE] = nonce.try_into().expect("split at the nonce's size");
        let aad = self.aad();
        self.cipher()
            .decrypt(
                &nonce.into(),
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| VaultError::WrongCode)
    }
}

fn derive(secret: &[u8; SECRET_SIZE], info: &[u8]) -> [u8; 32] {
    let mut okm = [0u8; 32];
    Hkdf::<Sha256>::new(None, secret)
        .expand(info, &mut okm)
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    okm
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn a_code_reads_back_as_it_was_written() {
        for secret in [[0u8; 16], [0xff; 16], *b"0123456789abcdef"] {
            let code = VaultCode::from_secret(secret);
            let text = code.to_text();
            assert_eq!(text.len(), CODE_LENGTH + 6, "{text}");
            assert_eq!(VaultCode::parse(&text).unwrap().secret(), secret);
        }
        assert_eq!(
            VaultCode::from_secret([0xff; 16]).to_text(),
            "ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZW"
        );
    }

    #[test]
    fn a_code_forgives_how_people_type_it() {
        let code = VaultCode::from_secret(*b"0123456789abcdef");
        let text = code.to_text();
        let sloppy = text.to_lowercase().replace('-', " ").replace('0', "o");
        assert_eq!(VaultCode::parse(&sloppy).unwrap(), code);
        assert_eq!(
            VaultCode::parse("1111-1111-1111-1111-1111-1111-10").unwrap(),
            VaultCode::parse("iiii-LLLL-1111-1111-1111-1111-10").unwrap()
        );
    }

    #[test]
    fn anything_else_is_not_a_code() {
        let text = VaultCode::from_secret([7; 16]).to_text();
        for bad in [
            "",
            &text[..text.len() - 1],
            &format!("{text}0"),
            "UUUU-UUUU-UUUU-UUUU-UUUU-UUUU-UU",
            "ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZ", // spare bits set
            "ÉZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-Z0",
        ] {
            assert_eq!(VaultCode::parse(bad), Err(VaultError::BadCode), "{bad}");
        }
    }

    #[test]
    fn the_derivations_match_the_protocol() {
        // Section 15.1, for the secret 00 01 … 0f. Computed once with
        // Python's hashlib/hmac (HKDF of RFC 5869) and kept here.
        let code = VaultCode::from_secret(std::array::from_fn(|i| i as u8));
        assert_eq!(code.to_text(), "000G-40R4-0M30-E209-185G-R38E-1W");
        assert_eq!(hex(&code.id()), ID_VECTOR);
        assert_eq!(hex(&derive(&code.secret(), KEY_INFO)), KEY_VECTOR);
    }

    const ID_VECTOR: &str = "2079b980fe62fb5d556bcd04252b5272df06a4cb49b62856f121d1a7a2c94561";
    const KEY_VECTOR: &str = "bf3d34d37b1b420ef4cb57c95e29cd69de59c0d859aadfea71c28b5e797e5831";

    #[test]
    fn a_vault_opens_with_its_code_only() {
        let code = VaultCode::from_secret([1; 16]);
        let sealed = code.seal(b"{\"name\":\"alex\"}", [2; 12]);
        assert_eq!(sealed[0], VERSION);
        assert_eq!(code.open(&sealed).unwrap(), b"{\"name\":\"alex\"}");

        let other = VaultCode::from_secret([3; 16]);
        assert_eq!(other.open(&sealed), Err(VaultError::WrongCode));
        let mut damaged = sealed.clone();
        *damaged.last_mut().unwrap() ^= 1;
        assert_eq!(code.open(&damaged), Err(VaultError::WrongCode));
        let mut newer = sealed.clone();
        newer[0] = 2;
        assert_eq!(code.open(&newer), Err(VaultError::UnsupportedVersion));
        assert_eq!(code.open(&[]), Err(VaultError::WrongCode));
        assert_eq!(code.open(&[VERSION, 0, 0]), Err(VaultError::WrongCode));
    }

    #[test]
    fn a_code_never_shows_in_a_debug_line() {
        let code = VaultCode::from_secret([9; 16]);
        assert!(!format!("{code:?}").contains(&code.to_text()));
    }
}
