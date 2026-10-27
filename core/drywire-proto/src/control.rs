//! Control packets (section 6): everything except audio.
//!
//! Builders return fixed-size arrays, because every control packet has one
//! exact size. Readers take a packet that [`parse`](crate::parse) accepted
//! with the matching type; they do not check the length again.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::Error;
use crate::header::{
    ANNOUNCE_SIZE, HEADER_SIZE, HELLO_SIZE, Header, KEEPALIVE_SIZE, PING_SIZE, PONG_SIZE,
    PacketType, SALT_SIZE, SIGNATURE_SIZE, read_u32, read_u64,
};

/// Size of a session secret (section 6.1).
pub const SECRET_SIZE: usize = 32;

/// The `hello` a client sends to bind its session to its current address
/// (section 6.1). `counter` must grow with every attempt; the current Unix
/// time in milliseconds does.
pub fn new_hello(session: u32, counter: u64, secret: &[u8; SECRET_SIZE]) -> [u8; HELLO_SIZE] {
    new_handshake(PacketType::Hello, session, counter, secret)
}

/// The relay's answer to a valid `hello` (section 6.2). Clients only need it
/// in tests; it is here so both directions are tested against the vectors.
pub fn new_hello_ack(session: u32, counter: u64, secret: &[u8; SECRET_SIZE]) -> [u8; HELLO_SIZE] {
    new_handshake(PacketType::HelloAck, session, counter, secret)
}

/// `hello` and `hello_ack` share one layout:
/// `header ‖ counter ‖ HMAC-SHA256(secret, header ‖ counter)`.
fn new_handshake(
    packet_type: PacketType,
    session: u32,
    counter: u64,
    secret: &[u8; SECRET_SIZE],
) -> [u8; HELLO_SIZE] {
    let mut b = [0u8; HELLO_SIZE];
    b[..HEADER_SIZE].copy_from_slice(&Header::new(packet_type, session).to_bytes());
    b[HEADER_SIZE..HEADER_SIZE + 8].copy_from_slice(&counter.to_be_bytes());
    let mac = handshake_mac(secret, &b[..HEADER_SIZE + 8])
        .finalize()
        .into_bytes();
    b[HEADER_SIZE + 8..].copy_from_slice(&mac);
    b
}

fn handshake_mac(secret: &[u8; SECRET_SIZE], header_and_counter: &[u8]) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC takes keys of any size");
    mac.update(header_and_counter);
    mac
}

/// Checks the MAC of a `hello` or `hello_ack`, in constant time, and returns
/// its counter. A client uses it on the `hello_ack` it receives, and also
/// checks that the counter is the one of its last `hello`.
pub fn verify_handshake(packet: &[u8], secret: &[u8; SECRET_SIZE]) -> Result<u64, Error> {
    let signed = &packet[..HEADER_SIZE + 8];
    handshake_mac(secret, signed)
        .verify_slice(&packet[HEADER_SIZE + 8..])
        .map_err(|_| Error::BadMac)?;
    Ok(read_u64(packet, HEADER_SIZE))
}

/// A `keepalive` (section 6.3).
pub fn new_keepalive(session: u32) -> [u8; KEEPALIVE_SIZE] {
    Header::new(PacketType::Keepalive, session).to_bytes()
}

/// A `ping` carrying the client's clock (section 6.4). The trailing eight
/// zero bytes make it as large as the `pong` it asks for.
pub fn new_ping(session: u32, number: u32, client_time: u64) -> [u8; PING_SIZE] {
    let header = Header {
        sequence: number,
        ..Header::new(PacketType::Ping, session)
    };
    let mut b = [0u8; PING_SIZE];
    b[..HEADER_SIZE].copy_from_slice(&header.to_bytes());
    b[HEADER_SIZE..HEADER_SIZE + 8].copy_from_slice(&client_time.to_be_bytes());
    b
}

/// The client time of a `ping`.
pub fn ping_client_time(packet: &[u8]) -> u64 {
    read_u64(packet, HEADER_SIZE)
}

/// The decoded payload of a `pong` (section 6.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pong {
    /// Copied from the ping's sequence field.
    pub number: u32,
    /// Copied from the ping.
    pub client_time: u64,
    /// Audio packets the relay received from this session.
    pub audio_received: u32,
    /// Audio packets the relay expected but never received.
    pub audio_lost: u32,
}

/// The relay's answer to a `ping`. Clients only need it in tests.
pub fn new_pong(session: u32, p: Pong) -> [u8; PONG_SIZE] {
    let header = Header {
        sequence: p.number,
        ..Header::new(PacketType::Pong, session)
    };
    let mut b = [0u8; PONG_SIZE];
    b[..HEADER_SIZE].copy_from_slice(&header.to_bytes());
    b[16..24].copy_from_slice(&p.client_time.to_be_bytes());
    b[24..28].copy_from_slice(&p.audio_received.to_be_bytes());
    b[28..32].copy_from_slice(&p.audio_lost.to_be_bytes());
    b
}

/// Reads a `pong`.
pub fn decode_pong(packet: &[u8]) -> Pong {
    Pong {
        number: read_u32(packet, 8),
        client_time: read_u64(packet, 16),
        audio_received: read_u32(packet, 24),
        audio_lost: read_u32(packet, 28),
    }
}

/// Builds an `announce` from its parts (section 6.7). Senders use
/// [`crypto::new_announce`](crate::crypto::new_announce), which also signs.
pub fn new_announce(
    session: u32,
    epoch: u8,
    salt: &[u8; SALT_SIZE],
    signature: &[u8; SIGNATURE_SIZE],
) -> [u8; ANNOUNCE_SIZE] {
    let header = Header {
        key_epoch: epoch,
        ..Header::new(PacketType::Announce, session)
    };
    let mut b = [0u8; ANNOUNCE_SIZE];
    b[..HEADER_SIZE].copy_from_slice(&header.to_bytes());
    b[HEADER_SIZE..HEADER_SIZE + SALT_SIZE].copy_from_slice(salt);
    b[HEADER_SIZE + SALT_SIZE..].copy_from_slice(signature);
    b
}

/// The salt and the signature of an `announce`, not yet verified.
pub fn announce_fields(packet: &[u8]) -> ([u8; SALT_SIZE], [u8; SIGNATURE_SIZE]) {
    let salt = packet[HEADER_SIZE..HEADER_SIZE + SALT_SIZE].try_into();
    let signature = packet[HEADER_SIZE + SALT_SIZE..ANNOUNCE_SIZE].try_into();
    (salt.expect("32 bytes"), signature.expect("64 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn hello_round_trips_and_rejects_another_secret() {
        let secret = [7u8; SECRET_SIZE];
        let hello = new_hello(42, 1000, &secret);
        assert_eq!(parse(&hello).unwrap().packet_type, PacketType::Hello);
        assert_eq!(verify_handshake(&hello, &secret), Ok(1000));
        assert_eq!(
            verify_handshake(&hello, &[8u8; SECRET_SIZE]),
            Err(Error::BadMac)
        );
    }

    #[test]
    fn a_hello_never_passes_as_a_hello_ack() {
        let secret = [7u8; SECRET_SIZE];
        let mut forged = new_hello(42, 1000, &secret);
        forged[1] = PacketType::HelloAck as u8;
        assert_eq!(verify_handshake(&forged, &secret), Err(Error::BadMac));
    }

    #[test]
    fn ping_is_as_large_as_pong() {
        assert_eq!(PING_SIZE, PONG_SIZE);
        let ping = new_ping(1, 9, 123);
        assert_eq!(ping_client_time(&ping), 123);
        assert_eq!(&ping[24..], &[0u8; 8]);
    }

    #[test]
    fn pong_round_trips() {
        let p = Pong {
            number: 9,
            client_time: 123,
            audio_received: 1000,
            audio_lost: 3,
        };
        assert_eq!(decode_pong(&new_pong(1, p)), p);
    }
}
