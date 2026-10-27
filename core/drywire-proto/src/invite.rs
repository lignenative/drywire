//! Invitation links (section 14).
//!
//! ```text
//! https://drywire.gg/i#<base64url(version ‖ code (16) ‖ space_key (32) ‖ instance)>
//! ```
//!
//! Everything is in the fragment, which browsers never send to a server:
//! the code and the space key appear in no access log and no `Referer`. The
//! link points to the project's domain even for a self-hosted instance; the
//! instance's own address is inside the payload. The server never sees the
//! space key: only clients build and read links.

use crate::crypto::KEY_SIZE;
use crate::login::{OriginError, canonical_origin};

/// The page every invitation link points to.
pub const BASE: &str = "https://drywire.gg/i#";
/// The first byte of the payload.
pub const VERSION: u8 = 1;
/// Size of an invitation code.
pub const CODE_SIZE: usize = 16;

/// What an invitation link carries.
#[derive(Clone, PartialEq, Eq)]
pub struct Invitation {
    /// The instance's canonical origin, such as `https://voice.example.org`.
    pub instance: String,
    /// The code the instance checks: it expires and has a maximum number of
    /// uses.
    pub code: [u8; CODE_SIZE],
    /// The secret every member of the space shares (section 8.1).
    pub space_key: [u8; KEY_SIZE],
}

/// Shows the instance only: the code and the key are secrets.
impl std::fmt::Debug for Invitation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Invitation")
            .field("instance", &self.instance)
            .finish_non_exhaustive()
    }
}

/// Why a link was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InviteError {
    /// It does not start with [`BASE`] (or `drywire://i#`).
    NotAnInvitation,
    /// The fragment is not base64url.
    BadEncoding,
    /// A payload version this client does not know: it needs an update.
    UnknownVersion(u8),
    /// Too short, or the instance is not valid UTF-8.
    BadPayload,
    /// The instance address is not a canonical origin.
    BadInstance(OriginError),
}

impl std::fmt::Display for InviteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InviteError::NotAnInvitation => {
                write!(f, "not an invitation link: it should start with {BASE}")
            }
            InviteError::BadEncoding | InviteError::BadPayload => {
                f.write_str("this invitation link is damaged: copy it again")
            }
            InviteError::UnknownVersion(v) => write!(
                f,
                "this invitation link is of a newer kind ({v}): update Drywire"
            ),
            InviteError::BadInstance(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for InviteError {}

impl Invitation {
    /// The link to share.
    pub fn to_link(&self) -> String {
        let mut payload = Vec::with_capacity(1 + CODE_SIZE + KEY_SIZE + self.instance.len());
        payload.push(VERSION);
        payload.extend_from_slice(&self.code);
        payload.extend_from_slice(&self.space_key);
        payload.extend_from_slice(self.instance.as_bytes());
        format!("{BASE}{}", base64url_encode(&payload))
    }

    /// Reads a link. Surrounding spaces are ignored, and the desktop
    /// scheme `drywire://i#…` is accepted as well.
    pub fn parse(link: &str) -> Result<Invitation, InviteError> {
        let link = link.trim();
        let fragment = link
            .strip_prefix(BASE)
            .or_else(|| link.strip_prefix("drywire://i#"))
            .ok_or(InviteError::NotAnInvitation)?;
        let payload = base64url_decode(fragment).ok_or(InviteError::BadEncoding)?;
        let (&version, rest) = payload.split_first().ok_or(InviteError::BadPayload)?;
        if version != VERSION {
            return Err(InviteError::UnknownVersion(version));
        }
        if rest.len() <= CODE_SIZE + KEY_SIZE {
            return Err(InviteError::BadPayload);
        }
        let (code, rest) = rest.split_at(CODE_SIZE);
        let (key, instance) = rest.split_at(KEY_SIZE);
        let instance = std::str::from_utf8(instance).map_err(|_| InviteError::BadPayload)?;
        let canonical = canonical_origin(instance).map_err(InviteError::BadInstance)?;
        if canonical != instance {
            return Err(InviteError::BadInstance(OriginError(format!(
                "instance address {instance:?} is not in canonical form"
            ))));
        }
        Ok(Invitation {
            instance: canonical,
            code: code.try_into().expect("split at CODE_SIZE"),
            space_key: key.try_into().expect("split at KEY_SIZE"),
        })
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Base64url without padding (RFC 4648 section 5).
pub fn base64url_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(ALPHABET[(n >> (18 - 6 * i)) as usize & 63] as char);
        }
    }
    out
}

/// Decodes base64url without padding; `None` if any character is outside
/// the alphabet or the length is impossible.
pub fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    if text.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    for chunk in text.as_bytes().chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let v = ALPHABET.iter().position(|&a| a == c)? as u32;
            n |= v << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
        // Unused low bits must be zero, so each text has one decoding.
        if bytes[chunk.len()..].iter().any(|&b| b != 0) {
            return None;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_round_trips_every_length() {
        let data: Vec<u8> = (0..=255).collect();
        for n in 0..data.len() {
            let text = base64url_encode(&data[..n]);
            assert!(!text.contains(['=', '+', '/']));
            assert_eq!(base64url_decode(&text).as_deref(), Some(&data[..n]));
        }
        assert_eq!(base64url_encode(b"\xfb\xff"), "-_8");
        assert_eq!(base64url_decode("AA"), Some(vec![0]));
        // A lone character, padding, the standard alphabet, and spare bits
        // that are not zero ("AB" would also decode to one byte).
        for bad in ["A", "AB=C", "A+BC", "AB"] {
            assert!(base64url_decode(bad).is_none(), "{bad}");
        }
    }

    fn sample() -> Invitation {
        Invitation {
            instance: "https://voice.example.org".into(),
            code: [7; CODE_SIZE],
            space_key: [9; KEY_SIZE],
        }
    }

    #[test]
    fn a_link_round_trips() {
        let link = sample().to_link();
        assert_eq!(Invitation::parse(&link), Ok(sample()));
        let desktop = link.replace(BASE, "drywire://i#");
        assert_eq!(Invitation::parse(&format!("  {desktop}\n")), Ok(sample()));
    }

    #[test]
    fn damaged_links_are_refused() {
        let link = sample().to_link();
        assert_eq!(
            Invitation::parse("https://example.org/i#abc"),
            Err(InviteError::NotAnInvitation)
        );
        // No instance after the key.
        let short = base64url_encode(&[VERSION; 1 + CODE_SIZE + KEY_SIZE]);
        assert_eq!(
            Invitation::parse(&format!("{BASE}{short}")),
            Err(InviteError::BadPayload)
        );
        assert_eq!(
            Invitation::parse(&format!("{link}*")),
            Err(InviteError::BadEncoding)
        );
        let mut newer = sample().to_link().into_bytes();
        newer[BASE.len()] = b'A' + 1; // version byte 0x04…
        assert!(matches!(
            Invitation::parse(std::str::from_utf8(&newer).unwrap()),
            Err(InviteError::UnknownVersion(_))
        ));
        let mut odd = sample();
        odd.instance = "https://Voice.example.org".into();
        assert!(matches!(
            Invitation::parse(&odd.to_link()),
            Err(InviteError::BadInstance(_))
        ));
    }

    #[test]
    fn debug_output_hides_the_secrets() {
        let shown = format!("{:?}", sample());
        assert!(shown.contains("voice.example.org"));
        assert!(!shown.contains("[7") && !shown.contains("9, 9"), "{shown}");
    }
}
