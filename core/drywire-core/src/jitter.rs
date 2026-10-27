//! The jitter buffer (spec section 7, "Reception"; ADR 0010): one per sender,
//! it holds frames just long enough to absorb the variation in their arrival
//! times, then hands them to the decoder in order, one per playback tick.
//!
//! How it works:
//!
//! - Frames are ordered by their **timestamp**, not their sequence number:
//!   timestamps keep advancing during silences and across key epochs, so a
//!   gap in timestamps is exactly a gap in sound.
//! - For every packet it records the **transit time**: arrival time minus the
//!   frame's position in the sender's timeline. Transit times differ only by
//!   a constant (the clocks' offset) plus the network delay, so the smallest
//!   recent transit is the "on time" baseline, and how far each packet lags
//!   behind it is its jitter.
//! - The **target delay** is the 95th percentile of that recent jitter plus a
//!   margin, never less than one frame, or two while the sender uses
//!   redundancy or has not shown yet whether it does.
//! - The target only changes **between talk spurts**. When a spurt starts,
//!   its first frame is scheduled at its on-time arrival plus the target, and
//!   the following frames play at the receiver's pace, one per tick. The
//!   silence before the next spurt is where the buffer re-synchronises,
//!   which also absorbs the clock drift between sender and receiver.
//! - Within a spurt, a frame missing at its tick is **concealed** (the
//!   decoder's PLC), and the buffer keeps waiting for it. If it then
//!   arrives, it plays one tick late: the spurt stretches by one frame,
//!   which is how a slower sender clock or a delay spike is absorbed. If a
//!   later frame arrives first, the missing one was lost: the buffer skips
//!   it, and the timing is unchanged. If more than two frames above target
//!   pile up, one is **dropped**. Everything is counted.
//!
//! The buffer knows nothing about Opus: frames are opaque bytes. It keeps no
//! clock either; every call takes the current time in microseconds, so tests
//! and `drywire-bench` drive it with simulated time.
//!
//! It runs in the playback callback, so it never allocates once made
//! (ADR 0017): frames are stored inline ([`FrameBytes`]) in a sorted queue
//! with room for [`JitterConfig::capacity`] of them, and the statistics
//! use a scratch buffer of their own.

use std::collections::VecDeque;
use std::fmt;
use std::ops::Deref;

use crate::opus::{FrameSize, MAX_FRAME_BYTES, SAMPLE_RATE};

/// One Opus frame of at most [`MAX_FRAME_BYTES`], stored inline rather than
/// on the heap, so that frames move from the network thread to the playback
/// callback, and are dropped there, without the allocator.
#[derive(Clone)]
pub struct FrameBytes {
    len: u16,
    bytes: [u8; MAX_FRAME_BYTES],
}

impl FrameBytes {
    /// A copy of `frame`, or `None` if it is longer than [`MAX_FRAME_BYTES`]
    /// (protocol section 7).
    pub fn new(frame: &[u8]) -> Option<FrameBytes> {
        let mut bytes = [0u8; MAX_FRAME_BYTES];
        bytes.get_mut(..frame.len())?.copy_from_slice(frame);
        Some(FrameBytes {
            len: frame.len() as u16,
            bytes,
        })
    }
}

impl Deref for FrameBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }
}

impl PartialEq for FrameBytes {
    fn eq(&self, other: &FrameBytes) -> bool {
        **self == **other
    }
}

impl Eq for FrameBytes {}

impl fmt::Debug for FrameBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

/// Settings of a [`JitterBuffer`]. The defaults are starting points that
/// `drywire-bench` measures, not tuned values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JitterConfig {
    /// The sender's frame size. A receiver reads it from the first frame
    /// (`opus::packet_samples`) and makes a new buffer if it ever changes.
    pub frame: FrameSize,
    /// Added to the 95th percentile of jitter to get the target delay.
    pub margin_us: u64,
    /// The smallest target delay. The spec's floor is one frame. While the
    /// sender uses redundancy, the floor is at least two frames, since the
    /// copy of a frame arrives one frame after it (spec section 7); so it
    /// is until the sender's packets say whether it does.
    pub min_target_us: u64,
    /// How many recent packets the jitter statistics cover.
    pub history: usize,
    /// After this long of consecutive concealment, the talk spurt is treated
    /// as over: its `end_of_spurt` packet was probably lost.
    pub max_conceal_us: u64,
    /// The most frames held at once; beyond it the oldest is dropped. Bounds
    /// memory against a sender that floods.
    pub capacity: usize,
    /// If a sender's timestamps move this much more, or less, than the time
    /// between two of its packets, its timeline was broken (a restart, or a
    /// silence longer than the 32-bit timestamp can tell), and the buffer
    /// starts over.
    pub max_jump_us: u64,
}

impl JitterConfig {
    /// Defaults for a sender using `frame`-sized frames.
    pub fn new(frame: FrameSize) -> JitterConfig {
        JitterConfig {
            frame,
            margin_us: 1_000,
            min_target_us: frame.micros(),
            history: 500,
            max_conceal_us: 60_000,
            capacity: 200,
            max_jump_us: 10_000_000,
        }
    }
}

/// One received audio packet, as the jitter buffer needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arrival {
    /// The packet's timestamp, in 48 kHz samples.
    pub timestamp: u32,
    /// Whether the packet had the `end_of_spurt` flag.
    pub end_of_spurt: bool,
    /// The primary Opus frame.
    pub primary: FrameBytes,
    /// The copy of the previous frame, if the packet carried one.
    pub redundant: Option<FrameBytes>,
}

/// What to play at one tick.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "the frame is inline on purpose: boxing it would allocate (ADR 0017)"
)]
pub enum Playout {
    /// No talk spurt is playing: play nothing (or silence) for this sender.
    Silence,
    /// Decode and play this frame.
    Frame {
        /// The Opus frame.
        frame: FrameBytes,
        /// How long the frame waited in the buffer: the latency it added.
        waited_us: u64,
        /// Whether it came from a redundant copy because its own packet was
        /// lost or late.
        recovered: bool,
    },
    /// The frame is missing: ask the decoder to conceal one frame.
    Conceal,
}

/// Counters kept by a [`JitterBuffer`] since it was made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JitterStats {
    /// Packets pushed.
    pub received: u64,
    /// Frames handed out for decoding, recovered ones included.
    pub played: u64,
    /// Frames played from a redundant copy.
    pub recovered: u64,
    /// Ticks filled by concealment.
    pub concealed: u64,
    /// Frames thrown away because too many piled up.
    pub dropped: u64,
    /// Packets that arrived after the buffer had played past them.
    pub late: u64,
    /// Packets whose frame was already buffered.
    pub duplicates: u64,
    /// Talk spurts started.
    pub spurts: u64,
    /// The target delay chosen for the current or last spurt.
    pub target_us: u64,
    /// Times the sender's timeline jumped and the buffer started over.
    pub discontinuities: u64,
}

/// A buffered frame.
struct Frame {
    bytes: FrameBytes,
    arrival_us: u64,
    end_of_spurt: bool,
    recovered: bool,
}

/// Buffered frames sorted by unwrapped timestamp, in a queue made once with
/// room for every frame the buffer may hold. Frames nearly always arrive
/// in order, so inserting is nearly always appending, and playing is
/// taking the first.
struct Frames(VecDeque<(i64, Frame)>);

impl Frames {
    fn with_capacity(capacity: usize) -> Frames {
        Frames(VecDeque::with_capacity(capacity))
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn clear(&mut self) {
        self.0.clear();
    }

    /// Where `timestamp` is, or would go.
    fn position(&self, timestamp: i64) -> Result<usize, usize> {
        self.0.binary_search_by_key(&timestamp, |(ts, _)| *ts)
    }

    fn first_timestamp(&self) -> Option<i64> {
        self.0.front().map(|(ts, _)| *ts)
    }

    /// The earliest frame at or before `timestamp`.
    fn first_until(&self, timestamp: i64) -> Option<i64> {
        self.first_timestamp().filter(|ts| *ts <= timestamp)
    }

    fn get(&self, timestamp: i64) -> Option<&Frame> {
        let i = self.position(timestamp).ok()?;
        Some(&self.0[i].1)
    }

    fn get_mut(&mut self, timestamp: i64) -> Option<&mut Frame> {
        let i = self.position(timestamp).ok()?;
        Some(&mut self.0[i].1)
    }

    fn contains(&self, timestamp: i64) -> bool {
        self.position(timestamp).is_ok()
    }

    /// Inserts a frame in its place. The caller checks first that its
    /// timestamp is not buffered yet; if it were, nothing would change.
    fn insert(&mut self, timestamp: i64, frame: Frame) {
        if let Err(i) = self.position(timestamp) {
            self.0.insert(i, (timestamp, frame));
        }
    }

    fn remove(&mut self, timestamp: i64) -> Option<Frame> {
        let i = self.position(timestamp).ok()?;
        self.0.remove(i).map(|(_, frame)| frame)
    }

    fn pop_first(&mut self) {
        self.0.pop_front();
    }
}

enum State {
    /// Between talk spurts. `start` caches the schedule of the next spurt
    /// once its first frame is known: (timestamp, time to start playing).
    Idle { start: Option<(i64, u64)> },
    /// Playing a spurt. `next` is the timestamp expected next; `waiting`
    /// counts the ticks concealed while it has not arrived.
    Playing { next: i64, waiting: i64 },
}

/// The jitter buffer of one sender.
pub struct JitterBuffer {
    config: JitterConfig,
    /// Buffered frames by unwrapped timestamp.
    frames: Frames,
    /// Recent transit times in microseconds, oldest first.
    transits: VecDeque<i64>,
    /// Where [`JitterBuffer::schedule`] sorts the recent jitter.
    scratch: Vec<u64>,
    /// The last raw timestamp seen and its unwrapped value, to turn 32-bit
    /// timestamps that wrap into ever-growing ones.
    unwrap_reference: Option<(u32, i64)>,
    /// Whether the sender currently adds a redundant copy to its packets,
    /// once a packet has said so. Until then the buffer assumes it does:
    /// the first packet of a spurt never has a copy, so a sender's first
    /// spurt would otherwise start with a one-frame target, too shallow
    /// for its copies (see `note_redundancy`).
    redundancy: Option<bool>,
    /// Arrival time and unwrapped timestamp of the last packet.
    last_arrival: Option<(u64, i64)>,
    /// Timestamp of the last frame played, dropped or given up on; anything
    /// at or before it arrives too late to play.
    last_played: Option<i64>,
    state: State,
    stats: JitterStats,
}

impl JitterBuffer {
    /// An empty buffer.
    pub fn new(config: JitterConfig) -> JitterBuffer {
        JitterBuffer {
            config,
            // A packet adds at most two frames (its own and the redundant
            // copy) before the oldest above capacity are dropped.
            frames: Frames::with_capacity(config.capacity + 2),
            transits: VecDeque::with_capacity(config.history),
            scratch: Vec::with_capacity(config.history),
            unwrap_reference: None,
            redundancy: None,
            last_arrival: None,
            last_played: None,
            state: State::Idle { start: None },
            stats: JitterStats {
                target_us: config.min_target_us,
                ..JitterStats::default()
            },
        }
    }

    /// The counters so far.
    pub fn stats(&self) -> JitterStats {
        self.stats
    }

    /// How many frames are buffered.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Whether no frame is buffered.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Forgets every frame and every timing, but keeps the counters. A
    /// receiver calls it when a new announcement shows the sender restarted;
    /// the buffer also does it by itself when the timestamps jump (see
    /// [`JitterConfig::max_jump_us`]).
    pub fn reset(&mut self) {
        self.frames.clear();
        self.transits.clear();
        self.unwrap_reference = None;
        self.redundancy = None;
        self.last_arrival = None;
        self.last_played = None;
        self.state = State::Idle { start: None };
    }

    /// Starts over with another configuration, counters included, reusing
    /// the memory already held: a receiver does it when a sender changes
    /// its frame size. It only allocates if the new configuration needs
    /// more room than the old one.
    pub fn reconfigure(&mut self, config: JitterConfig) {
        self.reset();
        self.frames.0.reserve(config.capacity + 2);
        self.transits.reserve(config.history);
        self.scratch.reserve(config.history);
        self.config = config;
        self.stats = JitterStats {
            target_us: config.min_target_us,
            ..JitterStats::default()
        };
    }

    /// Records a packet that arrived at `now_us`.
    pub fn push(&mut self, now_us: u64, arrival: Arrival) {
        self.stats.received += 1;
        let mut timestamp = self.unwrap(arrival.timestamp);
        if self.is_discontinuity(now_us, timestamp) {
            self.stats.discontinuities += 1;
            self.reset();
            timestamp = self.unwrap(arrival.timestamp);
        }
        self.last_arrival = Some((now_us, timestamp));
        self.note_redundancy(timestamp, arrival.redundant.is_some());
        self.record_transit(now_us, timestamp);

        if self.is_late(timestamp) {
            self.stats.late += 1;
        } else {
            match self.frames.get_mut(timestamp) {
                None => self.frames.insert(
                    timestamp,
                    Frame {
                        bytes: arrival.primary,
                        arrival_us: now_us,
                        end_of_spurt: arrival.end_of_spurt,
                        recovered: false,
                    },
                ),
                // The frame came first as a redundant copy: same bytes, ready
                // since the copy arrived. Its own packet only adds the flags.
                Some(frame) if frame.recovered => {
                    frame.recovered = false;
                    frame.end_of_spurt = arrival.end_of_spurt;
                }
                Some(_) => self.stats.duplicates += 1,
            }
        }

        // The redundant copy fills the slot just before, if it is still
        // missing and not yet due. It is the same frame its own packet would
        // have carried, so it never replaces anything.
        if let Some(frame) = arrival.redundant {
            let previous = timestamp - self.frame_samples();
            if !self.is_late(previous) && !self.frames.contains(previous) {
                self.frames.insert(
                    previous,
                    Frame {
                        bytes: frame,
                        arrival_us: now_us,
                        end_of_spurt: false,
                        recovered: true,
                    },
                );
            }
        }

        while self.frames.len() > self.config.capacity {
            self.frames.pop_first();
            self.stats.dropped += 1;
        }
    }

    /// Returns what to play at the tick happening at `now_us`. Call it once
    /// per frame period, at the pace of the sound card. Between spurts, when
    /// it returns [`Playout::Silence`], it may be called more often: it only
    /// starts the next spurt once its time has come.
    pub fn pop(&mut self, now_us: u64) -> Playout {
        if let State::Idle { start } = self.state {
            let Some(first) = self.frames.first_timestamp() else {
                return Playout::Silence;
            };
            // Schedule the spurt once per first frame; a frame that arrives
            // later but belongs before it gets a new schedule.
            let (scheduled, play_at) = match start {
                Some((ts, at)) if ts == first => (ts, at),
                _ => {
                    let at = self.schedule(first);
                    self.state = State::Idle {
                        start: Some((first, at)),
                    };
                    (first, at)
                }
            };
            if now_us < play_at {
                return Playout::Silence;
            }
            self.stats.spurts += 1;
            self.state = State::Playing {
                next: scheduled,
                waiting: 0,
            };
        }

        let State::Playing { next, waiting } = self.state else {
            unreachable!("the buffer is playing at this point");
        };
        let mut next = next;
        let mut waiting = waiting;

        // Too many frames waiting: drop the one due now, so the delay comes
        // back towards the target. One per tick at most, and never the last
        // frame of a spurt, since the silence after it resets the delay.
        if self.frames.len() > self.target_frames() + 2
            && let Some(due) = self.due(next, waiting)
            && self.frames.get(due).is_some_and(|f| !f.end_of_spurt)
        {
            self.frames.remove(due);
            self.stats.dropped += 1;
            self.last_played = Some(due);
            next = due + self.frame_samples();
            waiting = 0;
        }

        let Some(due) = self.due(next, waiting) else {
            // Nothing to play: conceal, and keep waiting for `next`.
            self.stats.concealed += 1;
            waiting += 1;
            let concealed_us = waiting as u64 * self.config.frame.micros();
            self.state = if concealed_us >= self.config.max_conceal_us {
                // The spurt is over; its end_of_spurt packet was probably
                // lost. Give up on every slot already concealed.
                self.last_played = Some(next + (waiting - 1) * self.frame_samples());
                State::Idle { start: None }
            } else {
                State::Playing { next, waiting }
            };
            return Playout::Conceal;
        };

        let buffered = self.frames.remove(due).expect("a due frame is buffered");
        self.last_played = Some(due);
        self.stats.played += 1;
        self.stats.recovered += u64::from(buffered.recovered);
        self.state = if buffered.end_of_spurt {
            State::Idle { start: None }
        } else {
            State::Playing {
                next: due + self.frame_samples(),
                waiting: 0,
            }
        };
        Playout::Frame {
            frame: buffered.bytes,
            waited_us: now_us.saturating_sub(buffered.arrival_us),
            recovered: buffered.recovered,
        }
    }

    /// The frame to play now, if any: `next`, or after `waiting` concealed
    /// ticks, the earliest frame whose slot has come. A frame found past
    /// `next` means the ones before it were lost, and the concealment
    /// already covered them.
    fn due(&self, next: i64, waiting: i64) -> Option<i64> {
        let last_due = next + waiting * self.frame_samples();
        self.frames.first_until(last_due)
    }

    fn frame_samples(&self) -> i64 {
        self.config.frame.samples() as i64
    }

    fn target_frames(&self) -> usize {
        self.stats.target_us.div_ceil(self.config.frame.micros()) as usize
    }

    fn is_late(&self, timestamp: i64) -> bool {
        self.last_played.is_some_and(|last| timestamp <= last)
    }

    /// Whether the timestamp moved much more, or less, than the time since
    /// the previous packet. Jitter and clock drift move them apart by
    /// milliseconds; only a broken timeline moves them apart by seconds.
    fn is_discontinuity(&self, now_us: u64, timestamp: i64) -> bool {
        let Some((at_us, previous)) = self.last_arrival else {
            return false;
        };
        let media_gap = media_us(timestamp - previous);
        let real_gap = now_us as i64 - at_us as i64;
        (media_gap - real_gap).unsigned_abs() > self.config.max_jump_us
    }

    /// Tracks whether the sender uses redundancy. A packet with a copy says
    /// yes. A packet without one says no only if its predecessor belongs to
    /// the same spurt, since the first packet of a spurt never has a copy.
    ///
    /// When the answer changes before a spurt starts playing, its schedule
    /// is made again with the right floor. This is how a sender's first
    /// spurt gets it right: scheduled with two frames while unknown, it
    /// stays at two frames when the second packet brings a copy, and comes
    /// back to one frame, about when the second packet arrives, when it
    /// does not. Should that second packet be lost, nothing can tell before
    /// the spurt starts, and a sender without redundancy keeps two frames
    /// for its first spurt.
    fn note_redundancy(&mut self, timestamp: i64, has_copy: bool) {
        let previous = timestamp - self.frame_samples();
        let same_spurt = match self.frames.get(previous) {
            Some(frame) => !frame.end_of_spurt,
            // Played already: the spurt still goes on only while playing.
            None => {
                self.last_played == Some(previous) && matches!(self.state, State::Playing { .. })
            }
        };
        let before = self.redundancy;
        if has_copy {
            self.redundancy = Some(true);
        } else if same_spurt {
            self.redundancy = Some(false);
        }
        if self.redundancy != before && matches!(self.state, State::Idle { .. }) {
            self.state = State::Idle { start: None };
        }
    }

    /// Turns a 32-bit timestamp into a 64-bit one that does not wrap, by
    /// assuming it is within ±2³¹ samples (12 hours) of the previous one.
    fn unwrap(&mut self, timestamp: u32) -> i64 {
        let unwrapped = match self.unwrap_reference {
            None => i64::from(timestamp),
            Some((raw, value)) => value + i64::from(timestamp.wrapping_sub(raw) as i32),
        };
        if self
            .unwrap_reference
            .is_none_or(|(_, value)| unwrapped > value)
        {
            self.unwrap_reference = Some((timestamp, unwrapped));
        }
        unwrapped
    }

    fn record_transit(&mut self, now_us: u64, timestamp: i64) {
        if self.transits.len() == self.config.history {
            self.transits.pop_front();
        }
        self.transits.push_back(now_us as i64 - media_us(timestamp));
    }

    /// Chooses the target delay from recent jitter and returns when the
    /// spurt starting at `first` should start playing.
    fn schedule(&mut self, first: i64) -> u64 {
        let base = self.transits.iter().copied().min().unwrap_or(0);
        self.scratch.clear();
        self.scratch
            .extend(self.transits.iter().map(|t| (t - base) as u64));
        self.scratch.sort_unstable();
        let p95 = percentile(&self.scratch, 95);
        let floor = if self.redundancy != Some(false) {
            self.config
                .min_target_us
                .max(2 * self.config.frame.micros())
        } else {
            self.config.min_target_us
        };
        self.stats.target_us = (p95 + self.config.margin_us).max(floor);

        // On-time arrival of the first frame, plus the target.
        let on_time = media_us(first) + base;
        (on_time + self.stats.target_us as i64).max(0) as u64
    }
}

/// The position of a timestamp in the sender's timeline, in microseconds.
fn media_us(timestamp: i64) -> i64 {
    timestamp * 1_000_000 / i64::from(SAMPLE_RATE)
}

/// The `p`th percentile of sorted values (nearest rank), or 0 if empty.
fn percentile(sorted: &[u64], p: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (sorted.len() * p).div_ceil(100).max(1);
    sorted[rank - 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: FrameSize = FrameSize::Ms5;
    const F_US: u64 = 5_000;
    const F_TS: u32 = 240;

    fn bytes(frame: &[u8]) -> FrameBytes {
        FrameBytes::new(frame).unwrap()
    }

    fn arrival(index: u32) -> Arrival {
        Arrival {
            timestamp: index * F_TS,
            end_of_spurt: false,
            primary: bytes(&[index as u8]),
            redundant: None,
        }
    }

    fn frame_of(p: &Playout) -> Option<u8> {
        match p {
            Playout::Frame { frame, .. } => Some(frame[0]),
            _ => None,
        }
    }

    /// Frames 0..n sent every 5 ms, arriving after `delay(i)` µs, with ticks
    /// every 5 ms offset by `phase`. Returns what each tick played.
    fn simulate(
        n: u32,
        delay: impl Fn(u32) -> u64,
        phase: u64,
        end: bool,
    ) -> (Vec<Playout>, JitterBuffer) {
        let mut jb = JitterBuffer::new(JitterConfig::new(FRAME));
        let mut events: Vec<(u64, Option<u32>)> = (0..n)
            .map(|i| (u64::from(i) * F_US + delay(i), Some(i)))
            .collect();
        let horizon = u64::from(n) * F_US + 200_000;
        events.extend((0..horizon / F_US).map(|k| (k * F_US + phase, None)));
        // Arrivals before ticks at the same instant.
        events.sort_by_key(|&(t, packet)| (t, packet.is_none()));
        let mut out = Vec::new();
        for (t, packet) in events {
            match packet {
                Some(i) => {
                    let mut a = arrival(i);
                    a.end_of_spurt = end && i == n - 1;
                    jb.push(t, a);
                }
                None => out.push(jb.pop(t)),
            }
        }
        (out, jb)
    }

    #[test]
    fn a_perfect_network_plays_every_frame_in_order_after_one_frame() {
        let (out, jb) = simulate(100, |_| 20_000, 1_000, true);
        let played: Vec<u8> = out.iter().filter_map(frame_of).collect();
        assert_eq!(played, (0..100).collect::<Vec<u8>>());

        let s = jb.stats();
        assert_eq!((s.played, s.concealed, s.dropped, s.late), (100, 0, 0, 0));
        assert_eq!(
            s.target_us, F_US,
            "no jitter: the target is the one-frame floor"
        );
        // Ticks fall 1 ms after arrivals: each frame waits the target, then
        // the 1 ms until the next tick.
        for p in &out {
            if let Playout::Frame { waited_us, .. } = p {
                assert_eq!(*waited_us, F_US + 1_000);
            }
        }
    }

    #[test]
    fn the_end_of_a_spurt_returns_to_silence() {
        let (out, jb) = simulate(10, |_| 20_000, 0, true);
        let last_frame = out.iter().rposition(|p| frame_of(p).is_some()).unwrap();
        assert!(out[last_frame + 1..].iter().all(|p| *p == Playout::Silence));
        assert_eq!(jb.stats().concealed, 0);
        assert!(jb.is_empty());
    }

    #[test]
    fn a_lost_frame_is_concealed_once() {
        let mut jb = JitterBuffer::new(JitterConfig::new(FRAME));
        let mut out = Vec::new();
        for i in 0..20u32 {
            let t = u64::from(i) * F_US;
            if i != 7 {
                jb.push(t, arrival(i));
            }
            out.push(jb.pop(t + 1_000));
        }
        let played: Vec<Option<u8>> = out.iter().map(frame_of).collect();
        let at = played.iter().position(|p| *p == Some(6)).unwrap();
        assert_eq!(out[at + 1], Playout::Conceal);
        assert_eq!(played[at + 2], Some(8));
        assert_eq!(jb.stats().concealed, 1);
    }

    #[test]
    fn a_late_frame_stretches_the_spurt_instead_of_being_lost() {
        // From frame 3 on, every frame arrives 7 ms later than before, as
        // after a delay spike that does not drain within the spurt.
        let delay = |i: u32| if i >= 3 { 27_000 } else { 20_000 };
        let (out, jb) = simulate(10, delay, 1_000, true);
        let played: Vec<u8> = out.iter().filter_map(frame_of).collect();
        assert_eq!(played, (0..10).collect::<Vec<u8>>(), "nothing is lost");

        // Frame 3 misses its tick by 1 ms: one concealment, then the rest
        // of the spurt plays one frame later than scheduled.
        let s = jb.stats();
        assert_eq!((s.concealed, s.late, s.dropped), (1, 0, 0), "{s:?}");
        let at = out.iter().position(|p| frame_of(p) == Some(2)).unwrap();
        assert_eq!(out[at + 1], Playout::Conceal);
        assert_eq!(frame_of(&out[at + 2]), Some(3));
    }

    #[test]
    fn a_redundant_copy_replaces_a_lost_frame() {
        // Spurt 1 tells the buffer the sender uses redundancy; spurt 2 then
        // starts with a two-frame target, deep enough for the copies.
        let mut jb = JitterBuffer::new(JitterConfig::new(FRAME));
        let packet = |i: u32, first: u32, last: u32| Arrival {
            redundant: (i > first).then(|| bytes(&[(i - 1) as u8])),
            end_of_spurt: i == last,
            ..arrival(i)
        };
        for i in 0..3u32 {
            jb.push(u64::from(i) * F_US, packet(i, 0, 2));
            jb.pop(u64::from(i) * F_US + 1_000);
        }
        let mut out = Vec::new();
        for i in 10..30u32 {
            let t = u64::from(i) * F_US;
            if i != 17 {
                jb.push(t, packet(i, 10, 29));
            }
            out.push(jb.pop(t + 1_000));
        }
        assert_eq!(jb.stats().target_us, 2 * F_US);
        // The end of spurt 1 plays first; then spurt 2, without a gap.
        let played: Vec<u8> = out
            .iter()
            .filter_map(frame_of)
            .filter(|f| *f >= 10)
            .collect();
        assert_eq!(&played[..15], &(10..25).collect::<Vec<u8>>()[..]);
        let s = jb.stats();
        assert_eq!((s.concealed, s.recovered), (0, 1));
        assert!(out.iter().any(|p| matches!(
            p,
            Playout::Frame {
                recovered: true,
                ..
            }
        )));
    }

    #[test]
    fn a_senders_first_spurt_gets_the_floor_its_redundancy_needs() {
        // Packets every 5 ms, ticks 1 ms after each. Returns the index of
        // the tick that played the first frame, and the target.
        let first_spurt = |redundancy: bool| {
            let mut jb = JitterBuffer::new(JitterConfig::new(FRAME));
            let mut first_played = None;
            for i in 0..10u32 {
                let t = u64::from(i) * F_US;
                jb.push(
                    t,
                    Arrival {
                        redundant: (redundancy && i > 0).then(|| bytes(&[(i - 1) as u8])),
                        ..arrival(i)
                    },
                );
                if frame_of(&jb.pop(t + 1_000)).is_some() {
                    first_played.get_or_insert(i);
                }
            }
            (first_played.unwrap(), jb.stats().target_us)
        };
        // Two frames from the start for a redundant sender: the copy of a
        // lost frame arrives one frame after it.
        assert_eq!(first_spurt(true), (2, 2 * F_US));
        // One frame for the others, known from their second packet.
        assert_eq!(first_spurt(false), (1, F_US));
    }

    #[test]
    fn a_frame_after_its_redundant_copy_is_not_a_duplicate() {
        let mut jb = JitterBuffer::new(JitterConfig::new(FRAME));
        jb.push(0, arrival(0));
        let mut second = arrival(2);
        second.redundant = Some(bytes(&[1]));
        jb.push(1, second);
        let mut first = arrival(1); // overtaken by the packet after it
        first.end_of_spurt = true;
        jb.push(2, first);

        assert_eq!(jb.stats().duplicates, 0);
        assert_eq!(frame_of(&jb.pop(F_US)), Some(0));
        assert_eq!(
            jb.pop(2 * F_US),
            Playout::Frame {
                frame: bytes(&[1]),
                waited_us: 2 * F_US - 1,
                recovered: false
            }
        );
        // Its own packet's end-of-spurt flag counted: frame 2 starts a new
        // talk spurt.
        assert_eq!(frame_of(&jb.pop(3 * F_US)), Some(2));
        assert_eq!(jb.stats().spurts, 2);
    }

    #[test]
    fn jitter_raises_the_target_at_the_next_spurt_only() {
        // Every fourth packet is 12 ms late.
        let delay = |i: u32| if i.is_multiple_of(4) { 32_000 } else { 20_000 };
        let mut jb = JitterBuffer::new(JitterConfig::new(FRAME));
        let mut t_tick = 500;
        let mut run_spurt = |jb: &mut JitterBuffer, first: u32, n: u32| {
            let mut pending: Vec<(u64, Arrival)> = (first..first + n)
                .map(|i| {
                    let mut a = arrival(i);
                    a.end_of_spurt = i == first + n - 1;
                    (u64::from(i) * F_US + delay(i), a)
                })
                .collect();
            pending.sort_by_key(|(t, _)| *t);
            let end = u64::from(first + n) * F_US + 100_000;
            let mut pending = pending.into_iter().peekable();
            while t_tick < end {
                while let Some((t, _)) = pending.peek()
                    && *t <= t_tick
                {
                    let (t, a) = pending.next().unwrap();
                    jb.push(t, a);
                }
                jb.pop(t_tick);
                t_tick += F_US;
            }
        };

        run_spurt(&mut jb, 0, 100);
        let first = jb.stats();
        assert_eq!(
            first.target_us, F_US,
            "first spurt: no history, floor target"
        );
        assert!(
            first.concealed > 0,
            "the floor cannot absorb 12 ms of jitter"
        );

        run_spurt(&mut jb, 200, 100);
        let second = jb.stats();
        assert_eq!(
            second.target_us,
            12_000 + 1_000,
            "p95 of jitter plus the margin"
        );
        assert_eq!(
            second.concealed, first.concealed,
            "the second spurt plays cleanly"
        );
        assert_eq!(second.late, first.late);
    }

    #[test]
    fn a_pile_up_drops_frames_back_to_target() {
        let mut jb = JitterBuffer::new(JitterConfig::new(FRAME));
        // A sender already known to send no copies, so the spurt starts
        // one frame after its first packet.
        jb.redundancy = Some(false);
        // Start a spurt on time, then a burst of 10 frames arrives at once.
        jb.push(0, arrival(0));
        assert_eq!(frame_of(&jb.pop(F_US)), Some(0));
        for i in 1..=10 {
            jb.push(F_US + 1, arrival(i));
        }
        let mut played = Vec::new();
        for k in 2..20 {
            played.extend(frame_of(&jb.pop(k * F_US)));
        }
        let s = jb.stats();
        assert!(s.dropped > 0, "{s:?}");
        assert_eq!(s.dropped + played.len() as u64, 10);
        assert!(
            played.windows(2).all(|w| w[0] < w[1]),
            "order is kept: {played:?}"
        );
    }

    #[test]
    fn late_and_duplicate_packets_are_counted_not_played() {
        let mut jb = JitterBuffer::new(JitterConfig::new(FRAME));
        jb.push(0, arrival(0));
        jb.push(1, arrival(0));
        assert_eq!(jb.stats().duplicates, 1);
        // Two frames: nothing has said yet that the sender sends no copies.
        assert_eq!(frame_of(&jb.pop(2 * F_US)), Some(0));
        jb.push(2 * F_US + 1, arrival(0));
        assert_eq!(jb.stats().late, 1);
        assert!(jb.is_empty());
    }

    #[test]
    fn a_lost_end_of_spurt_ends_the_spurt_after_max_conceal() {
        let (out, jb) = simulate(10, |_| 20_000, 0, false);
        let concealed = out.iter().filter(|p| **p == Playout::Conceal).count() as u64;
        assert_eq!(concealed * F_US, JitterConfig::new(FRAME).max_conceal_us);
        assert_eq!(out.last(), Some(&Playout::Silence));
        assert_eq!(jb.stats().spurts, 1);
    }

    #[test]
    fn timestamps_unwrap_across_2_to_the_32() {
        let mut jb = JitterBuffer::new(JitterConfig::new(FRAME));
        let start = u32::MAX - F_TS + 1; // the next frame wraps to 0
        for (k, ts) in [start, 0, F_TS].into_iter().enumerate() {
            let t = k as u64 * F_US;
            jb.push(
                t,
                Arrival {
                    timestamp: ts,
                    end_of_spurt: false,
                    primary: bytes(&[k as u8]),
                    redundant: None,
                },
            );
        }
        let played: Vec<u8> = (1..5).filter_map(|k| frame_of(&jb.pop(k * F_US))).collect();
        assert_eq!(played, vec![0, 1, 2]);
    }

    /// Plays a short spurt of `n` frames starting at `timestamp`, sent from
    /// `start_us` on a perfect 20 ms path, and returns what was played.
    fn spurt(jb: &mut JitterBuffer, start_us: u64, timestamp: u32, n: u32) -> Vec<u8> {
        let mut played = Vec::new();
        for k in 0..n + 10 {
            let t = start_us + u64::from(k) * F_US;
            if k < n {
                jb.push(
                    t + 20_000,
                    Arrival {
                        timestamp: timestamp.wrapping_add(k * F_TS),
                        end_of_spurt: k == n - 1,
                        primary: bytes(&[k as u8]),
                        redundant: None,
                    },
                );
            }
            played.extend(frame_of(&jb.pop(t + 21_000)));
        }
        played
    }

    #[test]
    fn a_silence_longer_than_the_timestamp_range_is_not_mistaken_for_lateness() {
        let mut jb = JitterBuffer::new(JitterConfig::new(FRAME));
        assert_eq!(spurt(&mut jb, 0, 1_000, 5), vec![0, 1, 2, 3, 4]);
        // 13 hours later, the timestamps went round more than half of 2³².
        let later = 13 * 3600 * 1_000_000;
        let timestamp = 1_000u32.wrapping_add((13u64 * 3600 * 48_000) as u32);
        assert_eq!(spurt(&mut jb, later, timestamp, 5), vec![0, 1, 2, 3, 4]);
        assert_eq!(jb.stats().late, 0);
    }

    #[test]
    fn a_sender_restart_is_detected_without_a_reset() {
        for jump in [-40_000_000i64, 40_000_000] {
            let mut jb = JitterBuffer::new(JitterConfig::new(FRAME));
            assert_eq!(spurt(&mut jb, 0, 1_000_000_000, 5), vec![0, 1, 2, 3, 4]);
            // One second later, timestamps restart from a random value.
            let restarted = (1_000_000_000 + jump) as u32;
            let played = spurt(&mut jb, 1_000_000, restarted, 5);
            assert_eq!(played, vec![0, 1, 2, 3, 4], "jump {jump}");
            assert_eq!(jb.stats().discontinuities, 1, "jump {jump}");
        }
    }

    #[test]
    fn a_spurt_right_after_another_keeps_the_redundancy_floor() {
        let mut jb = JitterBuffer::new(JitterConfig::new(FRAME));
        // Spurt 1 has copies; spurt 2 starts on the very next frame, as when
        // the gate reopens at once, so its first packet has no copy.
        for i in 0..6u32 {
            jb.push(
                u64::from(i) * F_US,
                Arrival {
                    end_of_spurt: i == 2,
                    redundant: (i != 0 && i != 3).then(|| bytes(&[(i - 1) as u8])),
                    ..arrival(i)
                },
            );
            jb.pop(u64::from(i) * F_US + 1_000);
        }
        assert_eq!(
            jb.redundancy,
            Some(true),
            "the first packet of a spurt says nothing"
        );
    }

    #[test]
    fn percentile_uses_the_nearest_rank() {
        let v: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile(&v, 95), 95);
        assert_eq!(percentile(&[7], 95), 7);
        assert_eq!(percentile(&[], 95), 0);
    }
}
