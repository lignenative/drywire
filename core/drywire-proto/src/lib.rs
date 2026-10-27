//! The Drywire wire protocol, as clients speak it.
//!
//! [`docs/protocol.md`](https://github.com/lignenative/drywire/blob/main/docs/protocol.md)
//! is the normative description of everything in this crate, and comments
//! refer to its section numbers. The Go relay implements the same document
//! without sharing any code; the binary test vectors in `vectors/` are the
//! contract between the two, and `tests/vectors.rs` checks this crate against
//! them.
//!
//! What lives where:
//!
//! - [`header`]: the 16-byte header, packet types, flags and sizes.
//! - [`parse()`]: the checks every packet goes through before anything else.
//! - [`control`]: `hello`, `hello_ack`, `keepalive`, `ping`, `pong` and the
//!   layout of `announce`.
//! - [`crypto`]: key derivation, audio encryption, signed announcements.
//! - [`payload`]: the plaintext layout inside an audio packet.
//! - [`replay`]: the window that drops duplicate audio.
//!
//! The crate does no I/O and keeps no clock: it turns bytes into values and
//! back, so it can be tested exhaustively.

#![forbid(unsafe_code)]

pub mod control;
pub mod crypto;
mod error;
pub mod header;
mod parse;
pub mod payload;
pub mod replay;

pub use error::Error;
pub use header::{Header, PacketType};
pub use parse::parse;
