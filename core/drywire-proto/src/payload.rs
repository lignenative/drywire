//! The plaintext inside an audio packet (section 7): one Opus frame, the
//! primary frame, optionally preceded by a copy of the previous one.

use crate::Error;
use crate::header::FLAG_REDUNDANT;

/// A decoded audio payload. Frames borrow from the decrypted plaintext.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioPayload<'a> {
    /// The previous frame, present when the redundant flag is set.
    pub redundant: Option<&'a [u8]>,
    /// The frame this packet is about.
    pub primary: &'a [u8],
}

impl AudioPayload<'_> {
    /// Returns the plaintext bytes and the flag bits they need.
    ///
    /// Without redundancy: `primary`. With it:
    /// `redundant length (2) ‖ redundant ‖ primary`.
    pub fn encode(&self) -> (Vec<u8>, u8) {
        match self.redundant {
            None => (self.primary.to_vec(), 0),
            Some(redundant) => {
                let length =
                    u16::try_from(redundant.len()).expect("an Opus frame fits in 1200 bytes");
                let mut b = Vec::with_capacity(2 + redundant.len() + self.primary.len());
                b.extend_from_slice(&length.to_be_bytes());
                b.extend_from_slice(redundant);
                b.extend_from_slice(self.primary);
                (b, FLAG_REDUNDANT)
            }
        }
    }

    /// Splits a decrypted plaintext according to the flags of its header.
    pub fn decode(flags: u8, plaintext: &[u8]) -> Result<AudioPayload<'_>, Error> {
        if flags & FLAG_REDUNDANT == 0 {
            if plaintext.is_empty() {
                return Err(Error::BadFraming);
            }
            return Ok(AudioPayload {
                redundant: None,
                primary: plaintext,
            });
        }
        let Some((length, rest)) = plaintext.split_first_chunk::<2>() else {
            return Err(Error::BadFraming);
        };
        let n = usize::from(u16::from_be_bytes(*length));
        // Both frames need at least one byte.
        if n < 1 || n >= rest.len() {
            return Err(Error::BadFraming);
        }
        let (redundant, primary) = rest.split_at(n);
        Ok(AudioPayload {
            redundant: Some(redundant),
            primary,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redundant_payload_round_trips() {
        let p = AudioPayload {
            redundant: Some(b"previous"),
            primary: b"current",
        };
        let (plaintext, flags) = p.encode();
        assert_eq!(flags, FLAG_REDUNDANT);
        assert_eq!(AudioPayload::decode(flags, &plaintext), Ok(p));
    }

    #[test]
    fn framing_errors() {
        let bad: &[(u8, &[u8])] = &[
            (0, b""),                        // no primary frame
            (FLAG_REDUNDANT, b"\x00"),       // length cut short
            (FLAG_REDUNDANT, b"\x00\x00a"),  // empty redundant frame
            (FLAG_REDUNDANT, b"\x00\x01a"),  // no primary frame left
            (FLAG_REDUNDANT, b"\x00\x05ab"), // length past the end
        ];
        for (flags, plaintext) in bad {
            assert_eq!(
                AudioPayload::decode(*flags, plaintext),
                Err(Error::BadFraming),
                "{plaintext:?}"
            );
        }
    }
}
