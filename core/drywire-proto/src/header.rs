//! The 16-byte header that starts every packet (section 3), packet types
//! (section 4), flags (section 5) and sizes.

/// The protocol version this crate speaks.
pub const VERSION: u8 = 1;

/// The versions this crate decodes: the current one and, once it exists, the
/// previous one (section 11).
pub const SUPPORTED_VERSIONS: &[u8] = &[VERSION];

/// Size of the header.
pub const HEADER_SIZE: usize = 16;
/// Largest datagram anyone sends or accepts (section 2).
pub const MAX_PACKET_SIZE: usize = 1200;
/// AES-GCM authentication tag at the end of an audio packet.
pub const TAG_SIZE: usize = 16;
/// Smallest audio packet: header, one byte of payload, tag.
pub const MIN_AUDIO_SIZE: usize = HEADER_SIZE + 1 + TAG_SIZE;
/// `hello` and `hello_ack`: header, counter, HMAC-SHA256.
pub const HELLO_SIZE: usize = HEADER_SIZE + 8 + 32;
/// `keepalive`: the header alone.
pub const KEEPALIVE_SIZE: usize = HEADER_SIZE;
/// `ping`: header, client time, eight zero bytes.
pub const PING_SIZE: usize = HEADER_SIZE + 8 + 8;
/// `pong`: header, client time, audio received, audio lost.
pub const PONG_SIZE: usize = HEADER_SIZE + 8 + 4 + 4;
/// Size of a sender salt (section 8.1).
pub const SALT_SIZE: usize = 32;
/// Size of an Ed25519 signature.
pub const SIGNATURE_SIZE: usize = 64;
/// `announce`: header, sender salt, signature.
pub const ANNOUNCE_SIZE: usize = HEADER_SIZE + SALT_SIZE + SIGNATURE_SIZE;

/// Flag of an audio packet: the payload also carries the previous frame.
pub const FLAG_REDUNDANT: u8 = 0x01;
/// Flag of an audio packet: the last one before the sender's gate closes.
pub const FLAG_END_OF_SPURT: u8 = 0x02;

/// The packet type, byte 1 of the header (section 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PacketType {
    /// One encrypted Opus frame, copied by the relay to the room.
    Audio = 0x01,
    /// A client binds its session to its address.
    Hello = 0x02,
    /// The relay confirms the binding.
    HelloAck = 0x03,
    /// A client stays bound while silent.
    Keepalive = 0x04,
    /// A client measures its round trip to the relay.
    Ping = 0x05,
    /// The relay answers a ping.
    Pong = 0x06,
    /// A client publishes the salt of a key epoch.
    Announce = 0x07,
}

impl PacketType {
    /// The type for a header byte, or `None` if version 1 does not define it.
    pub fn from_byte(b: u8) -> Option<PacketType> {
        Some(match b {
            0x01 => PacketType::Audio,
            0x02 => PacketType::Hello,
            0x03 => PacketType::HelloAck,
            0x04 => PacketType::Keepalive,
            0x05 => PacketType::Ping,
            0x06 => PacketType::Pong,
            0x07 => PacketType::Announce,
            _ => return None,
        })
    }

    /// The name used in `docs/protocol.md` and in the test vectors.
    pub fn name(self) -> &'static str {
        match self {
            PacketType::Audio => "audio",
            PacketType::Hello => "hello",
            PacketType::HelloAck => "hello_ack",
            PacketType::Keepalive => "keepalive",
            PacketType::Ping => "ping",
            PacketType::Pong => "pong",
            PacketType::Announce => "announce",
        }
    }

    /// Whether `size` is a valid total size for this type: audio has a
    /// minimum, every other type one exact size.
    pub fn size_is_valid(self, size: usize) -> bool {
        match self {
            PacketType::Audio => size >= MIN_AUDIO_SIZE,
            PacketType::Hello | PacketType::HelloAck => size == HELLO_SIZE,
            PacketType::Keepalive => size == KEEPALIVE_SIZE,
            PacketType::Ping => size == PING_SIZE,
            PacketType::Pong => size == PONG_SIZE,
            PacketType::Announce => size == ANNOUNCE_SIZE,
        }
    }
}

/// The decoded form of the 16 bytes that start every packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// Protocol version.
    pub version: u8,
    /// Packet type.
    pub packet_type: PacketType,
    /// Flags; only defined for audio.
    pub flags: u8,
    /// Key epoch of an audio packet or an announce, else 0.
    pub key_epoch: u8,
    /// The sender's session; never 0.
    pub session: u32,
    /// Audio sequence, ping number, or 0 (section 6).
    pub sequence: u32,
    /// Audio timestamp in 48 kHz samples, or 0.
    pub timestamp: u32,
}

impl Header {
    /// A header of the current version with every other field zero.
    pub fn new(packet_type: PacketType, session: u32) -> Header {
        Header {
            version: VERSION,
            packet_type,
            flags: 0,
            key_epoch: 0,
            session,
            sequence: 0,
            timestamp: 0,
        }
    }

    /// The 16 encoded bytes, in network byte order.
    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut b = [0u8; HEADER_SIZE];
        b[0] = self.version;
        b[1] = self.packet_type as u8;
        b[2] = self.flags;
        b[3] = self.key_epoch;
        b[4..8].copy_from_slice(&self.session.to_be_bytes());
        b[8..12].copy_from_slice(&self.sequence.to_be_bytes());
        b[12..16].copy_from_slice(&self.timestamp.to_be_bytes());
        b
    }

    /// Whether the redundant flag is set.
    pub fn is_redundant(&self) -> bool {
        self.flags & FLAG_REDUNDANT != 0
    }

    /// Whether the end-of-spurt flag is set.
    pub fn is_end_of_spurt(&self) -> bool {
        self.flags & FLAG_END_OF_SPURT != 0
    }
}

/// Reads a big-endian `u32` at `at`. The caller checked the length.
pub(crate) fn read_u32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

/// Reads a big-endian `u64` at `at`. The caller checked the length.
pub(crate) fn read_u64(b: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example of section 3, typed by hand so that the encoding is
    /// pinned independently of the generated test vectors.
    #[test]
    fn header_matches_protocol_example() {
        let want = [
            0x01, 0x01, 0x00, 0x00, 0x11, 0x22, 0x33, 0x44, 0x00, 0x00, 0x00, 0x07, 0x00, 0x01,
            0x2c, 0x00,
        ];
        let h = Header {
            sequence: 7,
            timestamp: 76800,
            ..Header::new(PacketType::Audio, 0x1122_3344)
        };
        assert_eq!(h.to_bytes(), want);
        assert_eq!(crate::parse(&[&want[..], &[0; 17]].concat()), Ok(h));
    }

    #[test]
    fn every_type_round_trips_through_its_byte() {
        for b in 0..=255u8 {
            if let Some(t) = PacketType::from_byte(b) {
                assert_eq!(t as u8, b);
            }
        }
        assert_eq!(PacketType::from_byte(0x00), None);
        assert_eq!(PacketType::from_byte(0x08), None);
    }
}
