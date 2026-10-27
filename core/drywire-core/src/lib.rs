//! Drywire's audio logic, shared by every client: the Opus codec, the voice
//! gate that decides when to send, and the jitter buffer that decides when to
//! play. Nothing here touches a sound card or a socket; the platform backends
//! and the network loop are built on top.
//!
//! The wire format and the encryption live in `drywire-proto`.

pub mod gate;
pub mod opus;
