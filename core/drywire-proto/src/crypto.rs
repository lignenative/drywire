//! End-to-end encryption (section 8): key derivation, sealing and opening
//! audio packets, and signed sender announcements.
//!
//! The relay never runs any of this. Only members of a space hold the
//! `space_key`, so only they can derive the keys that decrypt audio.

use aes_gcm::Aes256Gcm;
use aes_gcm::aead::{Aead, AeadInOut, KeyInit, Payload};
use ed25519_dalek::{Signature, Signer, Verifier};
/// The Ed25519 key types this module takes, re-exported so that users of the
/// crate need no direct dependency on `ed25519-dalek`.
pub use ed25519_dalek::{SigningKey, VerifyingKey};
use hkdf::Hkdf;
use sha2::Sha256;

use crate::Error;
use crate::control::{announce_fields, new_announce as announce_from_parts};
use crate::header::{ANNOUNCE_SIZE, HEADER_SIZE, Header, SALT_SIZE, TAG_SIZE, read_u32};
use crate::{PacketType, parse_as};

/// Size of every key: space key, room key, sender key (section 8.1).
pub const KEY_SIZE: usize = 32;
/// Size of a room identifier.
pub const ROOM_ID_SIZE: usize = 16;

const ROOM_INFO: &[u8] = b"drywire/room/v1";
const SENDER_INFO: &[u8] = b"drywire/sender/v1";
const ANNOUNCE_INFO: &[u8] = b"drywire/sender-announce/v1";

/// The key shared by every member of a room (section 8.2):
/// `HKDF-SHA256(ikm = space_key, salt = empty, info = "drywire/room/v1" ‖ room_id)`.
pub fn room_key(space_key: &[u8; KEY_SIZE], room_id: &[u8; ROOM_ID_SIZE]) -> [u8; KEY_SIZE] {
    hkdf(space_key, None, &[ROOM_INFO, room_id])
}

/// The key one sender uses during one key epoch (section 8.2):
/// `HKDF-SHA256(ikm = room_key, salt = sender_salt, info = "drywire/sender/v1" ‖ public key)`.
pub fn sender_key(
    room_key: &[u8; KEY_SIZE],
    sender_salt: &[u8; SALT_SIZE],
    sender_public_key: &VerifyingKey,
) -> [u8; KEY_SIZE] {
    hkdf(
        room_key,
        Some(sender_salt),
        &[SENDER_INFO, sender_public_key.as_bytes()],
    )
}

fn hkdf(ikm: &[u8], salt: Option<&[u8]>, info: &[&[u8]]) -> [u8; KEY_SIZE] {
    let mut okm = [0u8; KEY_SIZE];
    Hkdf::<Sha256>::new(salt, ikm)
        .expand_multi_info(info, &mut okm)
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    okm
}

/// The message a sender signs to publish the salt of a key epoch
/// (section 8.3). The room ID never travels on the wire but binds the
/// signature to one room.
pub fn announcement_message(
    room_id: &[u8; ROOM_ID_SIZE],
    session: u32,
    epoch: u8,
    sender_salt: &[u8; SALT_SIZE],
) -> Vec<u8> {
    let mut m = Vec::with_capacity(ANNOUNCE_INFO.len() + ROOM_ID_SIZE + 4 + 1 + SALT_SIZE);
    m.extend_from_slice(ANNOUNCE_INFO);
    m.extend_from_slice(room_id);
    m.extend_from_slice(&session.to_be_bytes());
    m.push(epoch);
    m.extend_from_slice(sender_salt);
    m
}

/// Builds a signed `announce` for one key epoch (section 6.7).
pub fn new_announce(
    identity: &SigningKey,
    room_id: &[u8; ROOM_ID_SIZE],
    session: u32,
    epoch: u8,
    sender_salt: &[u8; SALT_SIZE],
) -> [u8; ANNOUNCE_SIZE] {
    let message = announcement_message(room_id, session, epoch, sender_salt);
    let signature = identity.sign(&message).to_bytes();
    announce_from_parts(session, epoch, sender_salt, &signature)
}

/// Checks an `announce` and its signature, with the public key the receiver
/// knows for its session, and returns the salt.
pub fn verify_announce(
    sender_public_key: &VerifyingKey,
    room_id: &[u8; ROOM_ID_SIZE],
    packet: &[u8],
) -> Result<[u8; SALT_SIZE], Error> {
    let header = parse_as(packet, &[PacketType::Announce])?;
    let (salt, signature) = announce_fields(packet);
    let message = announcement_message(room_id, header.session, header.key_epoch, &salt);
    sender_public_key
        .verify(&message, &Signature::from_bytes(&signature))
        .map_err(|_| Error::BadSignature)?;
    Ok(salt)
}

/// The AES-GCM nonce of an audio packet: its sequence number, zero-padded on
/// the left to 12 bytes (section 8.4).
pub fn nonce(sequence: u32) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[8..].copy_from_slice(&sequence.to_be_bytes());
    n
}

/// A sender key ready to seal or open audio packets.
///
/// Preparing AES expands the key once; keep one `AudioKey` per sender and
/// epoch rather than rebuilding it for every packet.
pub struct AudioKey {
    cipher: Aes256Gcm,
}

impl AudioKey {
    /// Prepares a sender key.
    pub fn new(sender_key: &[u8; KEY_SIZE]) -> AudioKey {
        AudioKey {
            cipher: Aes256Gcm::new(sender_key.into()),
        }
    }

    /// Builds a complete audio packet: the header, then the payload encrypted
    /// with the header as additional authenticated data (section 8.4).
    pub fn seal(&self, header: &Header, payload: &[u8]) -> Vec<u8> {
        let mut packet = Vec::with_capacity(HEADER_SIZE + payload.len() + TAG_SIZE);
        self.seal_into(header, payload, &mut packet);
        packet
    }

    /// Like [`AudioKey::seal`], but writes the packet into `packet`, which is
    /// cleared first. Nothing is allocated when `packet` already has room
    /// for it, which the real-time capture path relies on.
    pub fn seal_into(&self, header: &Header, payload: &[u8], packet: &mut Vec<u8>) {
        let aad = header.to_bytes();
        packet.clear();
        packet.extend_from_slice(&aad);
        packet.extend_from_slice(payload);
        // Encrypted where it lies, then the tag after it.
        let tag = self
            .cipher
            .encrypt_inout_detached(
                &nonce(header.sequence).into(),
                &aad,
                (&mut packet[HEADER_SIZE..]).into(),
            )
            .expect("AES-GCM encrypts any payload under 64 GiB");
        packet.extend_from_slice(&tag);
    }

    /// Checks and decrypts an audio packet and returns its plaintext
    /// payload, or [`Error::BadTag`] if anything was changed.
    pub fn open(&self, packet: &[u8]) -> Result<Vec<u8>, Error> {
        parse_as(packet, &[PacketType::Audio])?;
        let (aad, sealed) = packet.split_at(HEADER_SIZE);
        let sequence = read_u32(aad, 8);
        self.cipher
            .decrypt(&nonce(sequence).into(), Payload { msg: sealed, aad })
            .map_err(|_| Error::BadTag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::TAG_SIZE;

    #[test]
    fn nonce_is_the_sequence_padded_on_the_left() {
        assert_eq!(nonce(0x0102_0304), [0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4]);
    }

    #[test]
    fn sealed_audio_opens_and_detects_tampering() {
        let key = AudioKey::new(&[9u8; KEY_SIZE]);
        let header = Header {
            sequence: 5,
            ..Header::new(PacketType::Audio, 1)
        };
        let packet = key.seal(&header, b"frame");
        let mut reused = vec![0xee; 3];
        key.seal_into(&header, b"frame", &mut reused);
        assert_eq!(reused, packet, "the same bytes, whatever the buffer held");
        assert_eq!(packet.len(), HEADER_SIZE + 5 + TAG_SIZE);
        assert_eq!(key.open(&packet).as_deref(), Ok(&b"frame"[..]));

        for i in [3, HEADER_SIZE, packet.len() - 1] {
            let mut tampered = packet.clone();
            tampered[i] ^= 1;
            assert_eq!(key.open(&tampered), Err(Error::BadTag), "byte {i}");
        }
    }

    #[test]
    fn readers_reject_packets_of_another_type_without_panicking() {
        let public = SigningKey::from_bytes(&[1u8; 32]).verifying_key();
        let key = AudioKey::new(&[9u8; KEY_SIZE]);
        let audio = key.seal(&Header::new(PacketType::Audio, 1), &[0; 34]); // 66 bytes
        let keepalive = crate::control::new_keepalive(1);
        let room = [0u8; ROOM_ID_SIZE];

        assert_eq!(
            verify_announce(&public, &room, &audio),
            Err(Error::UnknownType)
        );
        assert_eq!(key.open(&keepalive), Err(Error::UnknownType));
        assert_eq!(key.open(&[1, 1]), Err(Error::TooShort));
        assert_eq!(
            crate::control::verify_handshake(&audio, &[0; 32]),
            Err(Error::UnknownType)
        );
    }

    #[test]
    fn an_announce_for_another_room_does_not_verify() {
        let identity = SigningKey::from_bytes(&[1u8; 32]);
        let salt = [2u8; SALT_SIZE];
        let packet = new_announce(&identity, &[3u8; ROOM_ID_SIZE], 7, 0, &salt);
        let public = identity.verifying_key();
        assert_eq!(
            verify_announce(&public, &[3u8; ROOM_ID_SIZE], &packet),
            Ok(salt)
        );
        assert_eq!(
            verify_announce(&public, &[4u8; ROOM_ID_SIZE], &packet),
            Err(Error::BadSignature)
        );
    }
}
