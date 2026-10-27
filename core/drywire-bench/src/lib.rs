//! `drywire-bench` replays simulated network traces (jitter, loss, bursts,
//! delay spikes, clock drift) against the jitter buffer of `drywire-core`,
//! and reports the latency the buffer added, how often it concealed a
//! missing frame, and how often it dropped one.
//!
//! - [`network`]: a seeded random path and a simulated talker.
//! - [`simulate`]: runs one [`Scenario`](simulate::Scenario) and returns a
//!   [`Report`](simulate::Report).
//! - [`scenarios`]: the built-in scenarios, which `tests/regression.rs` also
//!   runs on every `make test`.
//!
//! Everything is deterministic: the same seed gives the same report, so a
//! change in the numbers always comes from a change in the code.

pub mod network;
pub mod scenarios;
pub mod simulate;
