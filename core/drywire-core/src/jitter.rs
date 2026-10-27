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
//!   margin, never less than one frame.
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

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, VecDeque};

use crate::opus::{FrameSize, SAMPLE_RATE};

/// Settings of a [`JitterBuffer`]. The defaults are starting points that
/// `drywire-bench` measures, not tuned values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JitterConfig {
    /// The sender's frame size. A receiver reads it from the first frame
    /// (`opus::packet_samples`) and makes a new buffer if it ever changes.
    pub frame: FrameSize,
    /// Added to the 95th percentile of jitter to get the target delay.
    pub margin_us: u64,
    /// The smallest target delay. The spec's floor is one frame.
    pub min_target_us: u64,
    /// How many recent packets the jitter statistics cover.
    pub history: usize,
    /// After this long of consecutive concealment, the talk spurt is treated
    /// as over: its `end_of_spurt` packet was probably lost.
    pub max_conceal_us: u64,
    /// The most frames held at once; beyond it the oldest is dropped. Bounds
    /// memory against a sender that floods.
    pub capacity: usize,
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
    pub primary: Vec<u8>,
    /// The copy of the previous frame, if the packet carried one.
    pub redundant: Option<Vec<u8>>,
}

/// What to play at one tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Playout {
    /// No talk spurt is playing: play nothing (or silence) for this sender.
    Silence,
    /// Decode and play this frame.
    Frame {
        /// The Opus frame.
        frame: Vec<u8>,
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
}

/// A buffered frame.
struct Frame {
    bytes: Vec<u8>,
    arrival_us: u64,
    end_of_spurt: bool,
    recovered: bool,
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
    frames: BTreeMap<i64, Frame>,
    /// Recent transit times in microseconds, oldest first.
    transits: VecDeque<i64>,
    /// The last raw timestamp seen and its unwrapped value, to turn 32-bit
    /// timestamps that wrap into ever-growing ones.
    unwrap_reference: Option<(u32, i64)>,
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
            frames: BTreeMap::new(),
            transits: VecDeque::with_capacity(config.history),
            unwrap_reference: None,
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

    /// Forgets every frame and every timing, but keeps the counters. Call it
    /// when the sender restarted (a new announcement replaced its key), since
    /// its timestamps start over from a new random value.
    pub fn reset(&mut self) {
        *self = JitterBuffer {
            stats: self.stats,
            ..JitterBuffer::new(self.config)
        };
    }

    /// Records a packet that arrived at `now_us`.
    pub fn push(&mut self, now_us: u64, arrival: Arrival) {
        self.stats.received += 1;
        let timestamp = self.unwrap(arrival.timestamp);
        self.record_transit(now_us, timestamp);

        if self.is_late(timestamp) {
            self.stats.late += 1;
        } else {
            match self.frames.entry(timestamp) {
                Entry::Vacant(slot) => {
                    slot.insert(Frame {
                        bytes: arrival.primary,
                        arrival_us: now_us,
                        end_of_spurt: arrival.end_of_spurt,
                        recovered: false,
                    });
                }
                // The frame came first as a redundant copy: same bytes, ready
                // since the copy arrived. Its own packet only adds the flags.
                Entry::Occupied(mut slot) if slot.get().recovered => {
                    let frame = slot.get_mut();
                    frame.recovered = false;
                    frame.end_of_spurt = arrival.end_of_spurt;
                }
                Entry::Occupied(_) => self.stats.duplicates += 1,
            }
        }

        // The redundant copy fills the slot just before, if it is still
        // missing and not yet due. It is the same frame its own packet would
        // have carried, so it never replaces anything.
        if let Some(frame) = arrival.redundant {
            let previous = timestamp - self.frame_samples();
            if !self.is_late(previous) && !self.frames.contains_key(&previous) {
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
    /// per frame period, at the pace of the sound card.
    pub fn pop(&mut self, now_us: u64) -> Playout {
        if let State::Idle { start } = self.state {
            let Some((&first, _)) = self.frames.first_key_value() else {
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
            && !self.frames[&due].end_of_spurt
        {
            self.frames.remove(&due);
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

        let buffered = self.frames.remove(&due).expect("a due frame is buffered");
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
        self.frames.range(..=last_due).next().map(|(&ts, _)| ts)
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
        let mut jitter: Vec<u64> = self.transits.iter().map(|t| (t - base) as u64).collect();
        jitter.sort_unstable();
        let p95 = percentile(&jitter, 95);
        self.stats.target_us = (p95 + self.config.margin_us).max(self.config.min_target_us);

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

    fn arrival(index: u32) -> Arrival {
        Arrival {
            timestamp: index * F_TS,
            end_of_spurt: false,
            primary: vec![index as u8],
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
        let mut jb = JitterBuffer::new(JitterConfig {
            min_target_us: 2 * F_US, // redundancy needs two frames of depth
            ..JitterConfig::new(FRAME)
        });
        let mut out = Vec::new();
        for i in 0..20u32 {
            let t = u64::from(i) * F_US;
            if i != 7 {
                let mut a = arrival(i);
                a.redundant = (i > 0).then(|| vec![(i - 1) as u8]);
                jb.push(t, a);
            }
            out.push(jb.pop(t + 1_000));
        }
        let played: Vec<u8> = out.iter().filter_map(frame_of).collect();
        assert_eq!(&played[..15], &(0..15).collect::<Vec<u8>>()[..]);
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
    fn a_frame_after_its_redundant_copy_is_not_a_duplicate() {
        let mut jb = JitterBuffer::new(JitterConfig::new(FRAME));
        jb.push(0, arrival(0));
        let mut second = arrival(2);
        second.redundant = Some(vec![1]);
        jb.push(1, second);
        let mut first = arrival(1); // overtaken by the packet after it
        first.end_of_spurt = true;
        jb.push(2, first);

        assert_eq!(jb.stats().duplicates, 0);
        assert_eq!(frame_of(&jb.pop(F_US)), Some(0));
        assert_eq!(
            jb.pop(2 * F_US),
            Playout::Frame {
                frame: vec![1],
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
        assert_eq!(frame_of(&jb.pop(F_US)), Some(0));
        jb.push(F_US + 1, arrival(0));
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
                    primary: vec![k as u8],
                    redundant: None,
                },
            );
        }
        let played: Vec<u8> = (1..5).filter_map(|k| frame_of(&jb.pop(k * F_US))).collect();
        assert_eq!(played, vec![0, 1, 2]);
    }

    #[test]
    fn percentile_uses_the_nearest_rank() {
        let v: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile(&v, 95), 95);
        assert_eq!(percentile(&[7], 95), 7);
        assert_eq!(percentile(&[], 95), 0);
    }
}
