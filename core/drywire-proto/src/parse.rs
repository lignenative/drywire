use crate::Error;
use crate::header::{
    HEADER_SIZE, Header, MAX_PACKET_SIZE, PacketType, SUPPORTED_VERSIONS, read_u32,
};

/// Checks a datagram's size and header and returns the decoded header.
///
/// It runs every check that needs no key, in the order of section 9: what
/// comes after the header is verified by
/// [`verify_handshake`](crate::control::verify_handshake) for `hello`, by
/// [`open_audio`](crate::crypto::open_audio) for audio and by
/// [`verify_announce`](crate::crypto::verify_announce) for `announce`.
pub fn parse(packet: &[u8]) -> Result<Header, Error> {
    if packet.len() < HEADER_SIZE {
        return Err(Error::TooShort);
    }
    if packet.len() > MAX_PACKET_SIZE {
        return Err(Error::TooLarge);
    }
    let version = packet[0];
    if !SUPPORTED_VERSIONS.contains(&version) {
        return Err(Error::UnsupportedVersion);
    }
    let packet_type = PacketType::from_byte(packet[1]).ok_or(Error::UnknownType)?;
    if !packet_type.size_is_valid(packet.len()) {
        return Err(Error::BadLength);
    }
    let session = read_u32(packet, 4);
    if session == 0 {
        return Err(Error::ZeroSession);
    }
    Ok(Header {
        version,
        packet_type,
        flags: packet[2],
        key_epoch: packet[3],
        session,
        sequence: read_u32(packet, 8),
        timestamp: read_u32(packet, 12),
    })
}

/// Like [`parse`], and also checks that the packet is one of `expected`.
/// Functions that read past the header use it, so that handing them a valid
/// packet of another type is an error, never a panic: a packet of another
/// type gives [`Error::UnknownType`], unknown to that function.
pub(crate) fn parse_as(packet: &[u8], expected: &[PacketType]) -> Result<Header, Error> {
    let header = parse(packet)?;
    if !expected.contains(&header.packet_type) {
        return Err(Error::UnknownType);
    }
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::new_keepalive;
    use crate::header::{MIN_AUDIO_SIZE, VERSION};

    fn audio(size: usize) -> Vec<u8> {
        let mut b = Header::new(PacketType::Audio, 1).to_bytes().to_vec();
        b.resize(size, 0);
        b
    }

    fn keepalive_with(i: usize, v: u8) -> Vec<u8> {
        let mut b = new_keepalive(1).to_vec();
        b[i] = v;
        b
    }

    #[test]
    fn errors_come_in_the_order_of_section_9() {
        let keepalive = new_keepalive(1).to_vec();
        let cases: Vec<(&str, Vec<u8>, Result<(), Error>)> = vec![
            ("valid keepalive", keepalive.clone(), Ok(())),
            ("valid smallest audio", audio(MIN_AUDIO_SIZE), Ok(())),
            ("valid largest audio", audio(MAX_PACKET_SIZE), Ok(())),
            ("empty", vec![], Err(Error::TooShort)),
            ("15 bytes", keepalive[..15].to_vec(), Err(Error::TooShort)),
            // Size is checked before anything else, even a bad version.
            (
                "10 bytes, bad version",
                keepalive_with(0, 9)[..10].to_vec(),
                Err(Error::TooShort),
            ),
            (
                "too large",
                audio(MAX_PACKET_SIZE + 1),
                Err(Error::TooLarge),
            ),
            (
                "version 0",
                keepalive_with(0, 0),
                Err(Error::UnsupportedVersion),
            ),
            (
                "version 2",
                keepalive_with(0, VERSION + 1),
                Err(Error::UnsupportedVersion),
            ),
            ("type 0", keepalive_with(1, 0), Err(Error::UnknownType)),
            ("type 8", keepalive_with(1, 8), Err(Error::UnknownType)),
            (
                "keepalive with a payload",
                [&keepalive[..], &[0]].concat(),
                Err(Error::BadLength),
            ),
            (
                "audio without payload",
                audio(MIN_AUDIO_SIZE - 1),
                Err(Error::BadLength),
            ),
            (
                "session 0",
                new_keepalive(0).to_vec(),
                Err(Error::ZeroSession),
            ),
        ];
        for (name, packet, want) in cases {
            assert_eq!(parse(&packet).map(|_| ()), want, "{name}");
        }
    }
}
