//! Runs one scenario: a talker sends frames over a simulated path, and a
//! receiver feeds them to the real [`JitterBuffer`] at the pace of its own
//! sound card.
//!
//! Time is the receiver's clock, in microseconds. The sender's clock runs
//! `drift_ppm` faster (or slower, if negative), as two sound cards' crystals
//! do, so its frames are sent slightly more (or less) often than the
//! receiver plays them.

use drywire_core::jitter::{Arrival, FrameBytes, JitterBuffer, JitterConfig, JitterStats, Playout};
use drywire_core::opus::FrameSize;

use crate::network::{Path, PathState, Rng, Talk};

/// Everything that defines one simulated run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scenario {
    /// Short name shown in reports.
    pub name: &'static str,
    /// The sender's frame size.
    pub frame: FrameSize,
    /// How long to simulate, in seconds.
    pub seconds: u64,
    /// The network path.
    pub path: Path,
    /// How the sender talks.
    pub talk: Talk,
    /// How much faster the sender's clock runs, in parts per million.
    pub drift_ppm: f64,
    /// Whether packets carry a copy of the previous frame.
    pub redundancy: bool,
    /// Seed of every random choice.
    pub seed: u64,
}

/// What a run measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    /// The scenario that was run.
    pub scenario: Scenario,
    /// Audio packets the sender sent.
    pub sent: u64,
    /// Packets the network lost.
    pub lost: u64,
    /// The jitter buffer's own counters.
    pub buffer: JitterStats,
    /// Latency the buffer added to played frames: median, 95th percentile,
    /// maximum, in microseconds.
    pub added_us: [u64; 3],
    /// Frames played out of order; must always be zero.
    pub out_of_order: u64,
}

impl Report {
    /// Concealed ticks as a share of the frames sent, in percent.
    pub fn concealed_percent(&self) -> f64 {
        percent(self.buffer.concealed, self.sent)
    }

    /// Dropped frames as a share of the frames sent, in percent.
    pub fn dropped_percent(&self) -> f64 {
        percent(self.buffer.dropped, self.sent)
    }
}

fn percent(n: u64, of: u64) -> f64 {
    if of == 0 {
        return 0.0;
    }
    100.0 * n as f64 / of as f64
}

/// One packet in flight. Its frames are made when it arrives, to keep the
/// thousands in flight small.
struct InFlight {
    arrives_us: u64,
    index: u64,
    timestamp: u32,
    end_of_spurt: bool,
    /// Whether it carries a copy of the previous frame.
    redundant: bool,
}

impl InFlight {
    /// The packet as the jitter buffer sees it. A frame's bytes are its
    /// index, so the receiver can check the order.
    fn arrival(&self) -> Arrival {
        let bytes = |k: u64| FrameBytes::new(&k.to_be_bytes()).expect("8 bytes fit");
        Arrival {
            timestamp: self.timestamp,
            end_of_spurt: self.end_of_spurt,
            primary: bytes(self.index),
            redundant: self.redundant.then(|| bytes(self.index - 1)),
        }
    }
}

/// Runs a scenario and reports what the jitter buffer did.
pub fn run(scenario: &Scenario) -> Report {
    let mut rng = Rng::new(scenario.seed);
    let frame = scenario.frame;
    let frame_us = frame.micros() as f64;
    let total_frames = scenario.seconds * 1_000_000 / frame.micros();

    // The sender: frame k of its timeline is captured at k frame periods of
    // its own clock. Its timestamps start at a random value, as the
    // protocol allows, which also exercises their wrapping.
    let sender_rate = 1.0 + scenario.drift_ppm / 1e6;
    let first_timestamp = rng.next_u64() as u32;
    let mut path = PathState::new(scenario.path);
    let mut packets = Vec::new();
    let mut sent = 0;
    let mut lost = 0;
    let mut talking_left = 0; // frames left in the current spurt
    let mut silent_left = rng.between(0, 20); // start a little after zero
    let mut has_previous = false;

    for k in 0..total_frames {
        if talking_left == 0 {
            if silent_left > 0 {
                silent_left -= 1;
                continue;
            }
            talking_left = frames_for(&mut rng, scenario.talk.spurt_ms, frame);
            silent_left = frames_for(&mut rng, scenario.talk.silence_ms, frame);
            has_previous = false; // never redundancy on a spurt's first packet
        }
        talking_left -= 1;

        let redundant = scenario.redundancy && has_previous;
        has_previous = true;
        sent += 1;

        let sent_us = (k as f64 * frame_us / sender_rate) as u64;
        match path.cross(&mut rng, sent_us) {
            Some(arrives_us) => packets.push(InFlight {
                arrives_us,
                index: k,
                timestamp: first_timestamp.wrapping_add((k * frame.samples() as u64) as u32),
                // The talker also stops when the simulation ends.
                end_of_spurt: talking_left == 0 || k == total_frames - 1,
                redundant,
            }),
            None => lost += 1,
        }
    }
    // Ties keep sending order, so the run is deterministic.
    packets.sort_by_key(|p| (p.arrives_us, p.index));

    // The receiver: one tick per frame period of its clock, starting at a
    // random phase, until every packet had a chance to play.
    let mut buffer = JitterBuffer::new(JitterConfig::new(frame));
    let mut waits = Vec::new();
    let mut last_played: Option<u64> = None;
    let mut out_of_order = 0;
    let end_us = packets.last().map_or(0, |p| p.arrives_us) + 1_000_000;
    let mut tick_us = rng.between(0, frame.micros() - 1);
    let mut incoming = packets.into_iter().peekable();
    while tick_us < end_us {
        while let Some(p) = incoming.next_if(|p| p.arrives_us <= tick_us) {
            buffer.push(p.arrives_us, p.arrival());
        }
        if let Playout::Frame {
            frame, waited_us, ..
        } = buffer.pop(tick_us)
        {
            let index = u64::from_be_bytes((*frame).try_into().expect("8-byte test frame"));
            if last_played.is_some_and(|last| index <= last) {
                out_of_order += 1;
            }
            last_played = Some(index);
            waits.push(waited_us);
        }
        tick_us += frame.micros();
    }

    waits.sort_unstable();
    Report {
        scenario: *scenario,
        sent,
        lost,
        buffer: buffer.stats(),
        added_us: [
            percentile(&waits, 50),
            percentile(&waits, 95),
            waits.last().copied().unwrap_or(0),
        ],
        out_of_order,
    }
}

/// A random duration in `range` milliseconds, as a whole number of frames.
fn frames_for(rng: &mut Rng, range_ms: (u64, u64), frame: FrameSize) -> u64 {
    let ms = rng.between(range_ms.0, range_ms.1);
    (ms * 1000).div_ceil(frame.micros()).max(1)
}

/// The `p`th percentile of sorted values (nearest rank), or 0 if empty.
fn percentile(sorted: &[u64], p: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[(sorted.len() * p).div_ceil(100).max(1) - 1]
}
