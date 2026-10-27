//! Playback: one jitter buffer and one Opus decoder per sender, mixed into
//! the samples the sound card asks for (spec section 7, "Reception").
//!
//! The [`Mixer`] runs where the sound card's output callback runs. Each
//! sender is a track that decodes one frame at a time, and pops its jitter
//! buffer when the previous frame has been played, so each track ticks at
//! the pace of its own frame size, all driven by the card's clock. Tracks
//! are summed, then softly clipped: a sum under half of full scale is left
//! exactly as it is.

use std::collections::BTreeMap;

use crate::inbound::Event;
use crate::jitter::{JitterBuffer, JitterConfig, JitterStats, Playout};
use crate::opus::{Decoder, FrameSize, OpusError, SAMPLE_RATE, packet_samples};

/// What one track is doing, for display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackStats {
    /// The sender's session.
    pub session: u32,
    /// The sender's frame size, once known.
    pub frame: Option<FrameSize>,
    /// The jitter buffer's counters, summed over every buffer this track had.
    pub jitter: JitterStats,
    /// Frames buffered right now.
    pub depth: usize,
    /// Frames Opus refused to decode, concealed instead.
    pub decode_errors: u64,
}

/// One sender's playback.
struct Track {
    session: u32,
    /// Made when the first frame shows the sender's frame size.
    buffer: Option<(FrameSize, JitterBuffer)>,
    decoder: Decoder,
    /// The decoded frame being played, and how much of it is played.
    pcm: Vec<f32>,
    played: usize,
    /// Counters of the buffers replaced after a frame size change.
    earlier: JitterStats,
    decode_errors: u64,
}

impl Track {
    fn new(session: u32) -> Result<Track, OpusError> {
        Ok(Track {
            session,
            buffer: None,
            decoder: Decoder::new()?,
            pcm: Vec::with_capacity(FrameSize::Ms10.samples()),
            played: 0,
            earlier: JitterStats::default(),
            decode_errors: 0,
        })
    }

    fn push(&mut self, now_us: u64, arrival: crate::jitter::Arrival) {
        let Some(frame) = packet_samples(&arrival.primary)
            .ok()
            .and_then(FrameSize::from_samples)
        else {
            self.decode_errors += 1;
            return;
        };
        // A sender that changes its frame size starts a new timeline.
        if self.buffer.as_ref().is_none_or(|(f, _)| *f != frame) {
            if let Some((_, old)) = &self.buffer {
                self.earlier = add(self.earlier, old.stats());
            }
            self.buffer = Some((frame, JitterBuffer::new(JitterConfig::new(frame))));
        }
        let (_, buffer) = self.buffer.as_mut().expect("just made");
        buffer.push(now_us, arrival);
    }

    /// Adds this track's next `out.len()` samples to `out`. `now_us` is the
    /// time of `out[0]`.
    fn render(&mut self, now_us: u64, out: &mut [f32]) {
        let Some((frame, _)) = self.buffer else {
            return;
        };
        let mut i = 0;
        while i < out.len() {
            if self.played == self.pcm.len() {
                let at = now_us + i as u64 * 1_000_000 / u64::from(SAMPLE_RATE);
                self.next_frame(frame, at);
            }
            let n = (self.pcm.len() - self.played).min(out.len() - i);
            for (o, s) in out[i..i + n]
                .iter_mut()
                .zip(&self.pcm[self.played..self.played + n])
            {
                *o += s;
            }
            self.played += n;
            i += n;
        }
    }

    /// Fills `pcm` with the next frame: decoded, concealed, or silence.
    fn next_frame(&mut self, frame: FrameSize, now_us: u64) {
        let (_, buffer) = self.buffer.as_mut().expect("checked by render");
        self.pcm.clear();
        self.pcm.resize(frame.samples(), 0.0);
        self.played = 0;
        let decoded = match buffer.pop(now_us) {
            Playout::Silence => return,
            Playout::Frame { frame: bytes, .. } => self.decoder.decode(&bytes, &mut self.pcm),
            Playout::Conceal => self.decoder.conceal(&mut self.pcm),
        };
        if decoded.is_err() {
            self.decode_errors += 1;
            // Keep the decoder's state moving, and play what it guesses.
            let _ = self.decoder.conceal(&mut self.pcm);
        }
    }

    fn stats(&self) -> TrackStats {
        TrackStats {
            session: self.session,
            frame: self.buffer.as_ref().map(|(f, _)| *f),
            jitter: self
                .buffer
                .as_ref()
                .map_or(self.earlier, |(_, b)| add(self.earlier, b.stats())),
            depth: self.buffer.as_ref().map_or(0, |(_, b)| b.len()),
            decode_errors: self.decode_errors,
        }
    }
}

/// Mixes every sender of a room into one mono 48 kHz stream.
#[derive(Default)]
pub struct Mixer {
    tracks: BTreeMap<u32, Track>,
}

impl Mixer {
    /// A mixer with no tracks yet; tracks appear with their first frame.
    pub fn new() -> Mixer {
        Mixer::default()
    }

    /// Handles what [`Inbound`](crate::inbound::Inbound) produced, at the
    /// time the packet arrived.
    pub fn push(&mut self, now_us: u64, event: Event) -> Result<(), OpusError> {
        match event {
            Event::Audio { session, arrival } => {
                let track = match self.tracks.entry(session) {
                    std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::btree_map::Entry::Vacant(e) => e.insert(Track::new(session)?),
                };
                track.push(now_us, arrival);
            }
            Event::Restarted { session } => {
                if let Some(track) = self.tracks.get_mut(&session)
                    && let Some((_, buffer)) = &mut track.buffer
                {
                    buffer.reset();
                }
            }
        }
        Ok(())
    }

    /// Writes the next `out.len()` samples of the mix. `now_us` is the time
    /// at which `out[0]` is due, on the clock the arrivals were stamped with.
    pub fn render(&mut self, now_us: u64, out: &mut [f32]) {
        out.fill(0.0);
        for track in self.tracks.values_mut() {
            track.render(now_us, out);
        }
        for s in out.iter_mut() {
            *s = soft_clip(*s);
        }
    }

    /// What each track is doing, by session.
    pub fn stats(&self) -> Vec<TrackStats> {
        self.tracks.values().map(Track::stats).collect()
    }
}

/// Leaves `x` unchanged up to half of full scale, then bends it smoothly
/// towards ±1 (the slope stays continuous at the knee), so a loud sum of
/// voices saturates gently instead of clipping hard.
pub fn soft_clip(x: f32) -> f32 {
    const KNEE: f32 = 0.5;
    let magnitude = x.abs();
    if magnitude <= KNEE {
        return x;
    }
    let bent = KNEE + (1.0 - KNEE) * ((magnitude - KNEE) / (1.0 - KNEE)).tanh();
    bent.copysign(x)
}

fn add(a: JitterStats, b: JitterStats) -> JitterStats {
    JitterStats {
        received: a.received + b.received,
        played: a.played + b.played,
        recovered: a.recovered + b.recovered,
        concealed: a.concealed + b.concealed,
        dropped: a.dropped + b.dropped,
        late: a.late + b.late,
        duplicates: a.duplicates + b.duplicates,
        spurts: a.spurts + b.spurts,
        target_us: b.target_us,
        discontinuities: a.discontinuities + b.discontinuities,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jitter::Arrival;
    use crate::opus::Encoder;

    #[test]
    fn soft_clip_is_transparent_below_the_knee_and_bounded_above() {
        for x in [-0.5, -0.1, 0.0, 0.25, 0.5] {
            assert_eq!(soft_clip(x), x);
        }
        assert!(soft_clip(0.6) > 0.59 && soft_clip(0.6) < 0.6);
        assert!(soft_clip(10.0) <= 1.0);
        assert_eq!(soft_clip(-3.0), -soft_clip(3.0));
    }

    #[test]
    fn two_senders_are_summed() {
        let frame = FrameSize::Ms5;
        let mut mixer = Mixer::new();
        let tone = |f: f32| -> Vec<f32> {
            (0..frame.samples())
                .map(|i| 0.1 * (i as f32 * f).sin())
                .collect()
        };
        // Frame k of each sender arrives at k × 5 ms.
        let mut arrivals = Vec::new();
        for (session, f) in [(1, 0.05), (2, 0.11)] {
            let mut encoder = Encoder::new(frame, 64_000).unwrap();
            for k in 0..20u32 {
                let arrival = Arrival {
                    timestamp: k * frame.samples() as u32,
                    end_of_spurt: k == 19,
                    primary: encoder.encode(&tone(f)).unwrap(),
                    redundant: None,
                };
                arrivals.push((u64::from(k) * 5_000, Event::Audio { session, arrival }));
            }
        }
        arrivals.sort_by_key(|(at, _)| *at);
        let mut arrivals = arrivals.into_iter().peekable();

        // Render in device-sized chunks that are not a multiple of a frame.
        let mut energy = 0.0;
        let mut out = [0.0f32; 64];
        let mut t = 0;
        while t < 150_000 {
            while let Some((at, event)) = arrivals.next_if(|(at, _)| *at <= t) {
                mixer.push(at, event).unwrap();
            }
            mixer.render(t, &mut out);
            energy += out.iter().map(|x| x * x).sum::<f32>();
            t += 64 * 1_000_000 / 48_000;
        }
        assert!(energy > 1.0, "both tracks played: energy {energy}");
        let stats = mixer.stats();
        assert_eq!(stats.len(), 2);
        for track in stats {
            assert_eq!(track.jitter.played, 20, "{track:?}");
            assert_eq!(track.decode_errors, 0);
        }
    }
}
