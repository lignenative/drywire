use std::fmt;

/// Why a packet is invalid (section 9).
///
/// Each variant has a stable code, the same string the Go relay uses and the
/// test vectors contain. Codes are for logs and counters; they are never sent
/// on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Error {
    /// Fewer than 16 bytes.
    TooShort,
    /// More than 1200 bytes.
    TooLarge,
    /// Version byte not supported by this implementation.
    UnsupportedVersion,
    /// Type not defined in this version.
    UnknownType,
    /// Control packet of the wrong size, or audio packet under 33 bytes.
    BadLength,
    /// Session is 0.
    ZeroSession,
    /// MAC of a `hello` or `hello_ack` does not verify.
    BadMac,
    /// AES-GCM decryption fails.
    BadTag,
    /// Audio payload breaks the rules of section 7.
    BadFraming,
    /// Signature of an `announce` does not verify.
    BadSignature,
}

impl Error {
    /// The code of section 9, as written in logs and in the test vectors.
    pub fn code(self) -> &'static str {
        match self {
            Error::TooShort => "too_short",
            Error::TooLarge => "too_large",
            Error::UnsupportedVersion => "unsupported_version",
            Error::UnknownType => "unknown_type",
            Error::BadLength => "bad_length",
            Error::ZeroSession => "zero_session",
            Error::BadMac => "bad_mac",
            Error::BadTag => "bad_tag",
            Error::BadFraming => "bad_framing",
            Error::BadSignature => "bad_signature",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for Error {}
