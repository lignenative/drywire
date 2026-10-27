//! Drywire's client logic, shared by every client: the Opus codec, the voice
//! gate that decides when to send, the jitter buffer that decides when to
//! play, and the pieces of a call built on them.
//!
//! - [`config`]: the phase 1 client configuration file (ADR 0006).
//! - [`sender`]: microphone frames to sealed packets.
//! - [`inbound`]: packets from the relay to checked, decrypted frames.
//! - [`mixer`]: one jitter buffer and decoder per sender, mixed for the
//!   sound card.
//! - [`link`]: `hello`, `ping` and `keepalive` with the relay.
//!
//! None of these touches a sound card or a socket; the platform backends and
//! the network loop are built on top. The wire format and the encryption
//! live in `drywire-proto`.

pub mod config;
pub mod gate;
pub mod inbound;
pub mod jitter;
pub mod link;
pub mod mixer;
pub mod opus;
pub mod sender;
