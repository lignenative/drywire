//! Playback: one jitter buffer and one Opus decoder per sender, mixed into
//! the samples the sound card asks for (spec section 7, "Reception").
//!
//! The [`Mixer`] runs where the sound card's output callback runs. Each
//! sender is a track that decodes one frame at a time, and pops its jitter
//! buffer when the previous frame has been played, so each track ticks at
//! the pace of its own frame size, all driven by the card's clock. Between
//! talk spurts a track ticks every [`IDLE_TICK`] samples instead, so a
//! spurt starts within half a millisecond of the time its jitter buffer
//! scheduled, not up to a whole frame later. Tracks are summed, then
//! softly clipped: a sum under half of full scale is left exactly as it
//! is.
//!
//! The playback callback never allocates (ADR 0017), and making a track
//! does: it creates an Opus decoder and a jitter buffer. So a call makes
//! one track per member before playback starts ([`Mixer::add_track`]),
//! and while it runs, the network thread makes a spare track
//! ([`Mixer::make_slot`]) when someone joins and passes it to playback
//! ([`Mixer::add_slot`]). A member who leaves frees their track
//! ([`Event::Left`]), which the next sender reuses: reset in place, never
//! freed while the call lasts. A track's jitter buffer is also kept, and
//! reconfigured in place, when its sender changes frame size.

use crate::inbound::Event;
use crate::jitter::{Arrival, JitterBuffer, JitterConfig, JitterStats, Playout};
use crate::opus::{Decoder, FrameSize, OpusError, SAMPLE_RATE, packet_samples};

/// How often, in samples, an idle track asks its jitter buffer whether a
/// spurt starts: 0.5 ms. Asking costs a comparison; waiting for a whole
/// frame instead would delay each spurt, and so every frame in it, by half
/// a frame on average.
pub const IDLE_TICK: usize = 24;

/// What one track is doing, for display.
#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// Whether the track is playing the sender's voice (decoded or
    /// concealed) rather than silence between talk spurts: a "speaking"
    /// light.
    pub speaking: bool,
    /// The volume it is played at: 1 is as sent.
    pub volume: f32,
}

/// The loudest volume a sender can be played at: 200 % (spec section 10).
pub const MAX_VOLUME: f32 = 2.0;

/// How far the gain moves per sample towards a new volume: a change of 1
/// (100 %) takes 5 ms. A step at once would click, at every move of a
/// volume slider.
const GAIN_STEP: f32 = 1.0 / 240.0;

/// One sender's playback.
struct Track {
    /// The sender's session, or [`FREE`] for a track nobody uses.
    session: u32,
    /// The sender's frame size, known from its first frame. Until then the
    /// buffer is empty and the track silent.
    frame: Option<FrameSize>,
    buffer: JitterBuffer,
    decoder: Decoder,
    /// The decoded frame being played, and how much of it is played.
    pcm: Vec<f32>,
    played: usize,
    /// Counters of the buffers replaced after a frame size change.
    earlier: JitterStats,
    decode_errors: u64,
    /// Whether the frame being played is voice, not idle silence.
    speaking: bool,
    /// The volume asked for.
    volume: f32,
    /// What every sample is multiplied by: `volume`, or on its way there.
    gain: f32,
}

/// The session of a free track: 0 is never a valid session (protocol
/// section 3).
const FREE: u32 = 0;

impl Track {
    fn new(session: u32) -> Result<Track, OpusError> {
        Ok(Track {
            session,
            frame: None,
            // Reconfigured for the real frame size before the first push.
            buffer: JitterBuffer::new(JitterConfig::new(FrameSize::Ms5)),
            decoder: Decoder::new()?,
            pcm: Vec::with_capacity(FrameSize::Ms10.samples()),
            played: 0,
            earlier: JitterStats::default(),
            decode_errors: 0,
            speaking: false,
            volume: 1.0,
            gain: 1.0,
        })
    }

    /// Makes a free track, or one whose sender left, ready for `session`,
    /// as if new and without allocating.
    fn reuse(&mut self, session: u32) {
        self.session = session;
        self.frame = None;
        self.buffer.reset();
        self.decoder.reset();
        self.pcm.clear();
        self.played = 0;
        self.earlier = JitterStats::default();
        self.decode_errors = 0;
        self.speaking = false;
        self.volume = 1.0;
        self.gain = 1.0;
    }

    fn push(&mut self, now_us: u64, arrival: Arrival, min_target_us: u64) {
        let Some(frame) = packet_samples(&arrival.primary)
            .ok()
            .and_then(FrameSize::from_samples)
        else {
            self.decode_errors += 1;
            return;
        };
        // A sender that changes its frame size starts a new timeline.
        if self.frame != Some(frame) {
            if self.frame.is_some() {
                self.earlier = add(self.earlier, self.buffer.stats());
            }
            let mut config = JitterConfig::new(frame);
            config.min_target_us = config.min_target_us.max(min_target_us);
            self.buffer.reconfigure(config);
            self.frame = Some(frame);
        }
        self.buffer.push(now_us, arrival);
    }

    /// Adds this track's next `out.len()` samples to `out`. `now_us` is the
    /// time of `out[0]`.
    fn render(&mut self, now_us: u64, out: &mut [f32]) {
        let Some(frame) = self.frame else {
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
                if self.gain != self.volume {
                    let gap = self.volume - self.gain;
                    self.gain = if gap.abs() <= GAIN_STEP {
                        self.volume
                    } else {
                        self.gain + GAIN_STEP.copysign(gap)
                    };
                }
                *o += s * self.gain;
            }
            self.played += n;
            i += n;
        }
    }

    /// Fills `pcm` with the next frame: decoded, concealed, or silence.
    /// Silence lasts one [`IDLE_TICK`], so the next spurt is asked for
    /// again soon.
    fn next_frame(&mut self, frame: FrameSize, now_us: u64) {
        self.pcm.clear();
        self.pcm.resize(frame.samples(), 0.0);
        self.played = 0;
        self.speaking = true;
        let decoded = match self.buffer.pop(now_us) {
            Playout::Silence => {
                self.speaking = false;
                self.pcm.truncate(IDLE_TICK);
                return;
            }
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
        let known = self.frame.is_some();
        TrackStats {
            session: self.session,
            frame: self.frame,
            jitter: if known {
                add(self.earlier, self.buffer.stats())
            } else {
                self.earlier
            },
            depth: self.buffer.len(),
            decode_errors: self.decode_errors,
            speaking: self.speaking,
            volume: self.volume,
        }
    }
}

/// A track made ahead of time, off the playback thread, for a running
/// mixer to take without allocating ([`Mixer::add_slot`]).
pub struct Slot(Box<Track>);

/// Mixes every sender of a room into one mono 48 kHz stream.
#[derive(Default)]
pub struct Mixer {
    /// Boxed, so a slot handed in by another thread moves in as a
    /// pointer: unboxing it here would free the box on the playback thread.
    #[expect(clippy::vec_box, reason = "see above, and ADR 0017")]
    tracks: Vec<Box<Track>>,
    min_target_us: u64,
    /// The counters of the senders who left, summed.
    departed: JitterStats,
    /// The volume of each sender who has one other than 1, by session,
    /// kept apart from the tracks so that a volume set before a sender's
    /// first frame applies when their track is assigned. Its room is
    /// reserved with the tracks', and it never grows past it.
    volumes: Vec<(u32, f32)>,
}

impl Mixer {
    /// A mixer with no tracks yet: add them with [`Mixer::add_track`], or
    /// let each appear with its sender's first frame.
    pub fn new() -> Mixer {
        Mixer::default()
    }

    /// A mixer whose jitter buffers never aim below `min_target_us`: it
    /// raises their one-frame floor when it is larger. Raising it trades latency for fewer
    /// gaps on a network, or a machine, that stalls now and then.
    pub fn with_min_target(min_target_us: u64) -> Mixer {
        Mixer {
            min_target_us,
            ..Mixer::default()
        }
    }

    /// Makes the track of `session` now, rather than when its first frame
    /// arrives: making a track allocates, so a call makes every member's
    /// before playback starts. A track that exists already is kept; a free
    /// one is used before a new one is made.
    pub fn add_track(&mut self, session: u32) -> Result<(), OpusError> {
        self.track(session)?;
        Ok(())
    }

    /// Makes room for `n` more tracks, so that [`Mixer::add_slot`] never
    /// grows the list while playback runs.
    pub fn reserve(&mut self, n: usize) {
        self.tracks.reserve(n);
        self.volumes.reserve(n);
    }

    /// Plays `session` at `volume` (1 is as sent; clamped to 0 through
    /// [`MAX_VOLUME`]), from now on, and on the track the sender gets
    /// later if they have none yet. It does not allocate: past the room
    /// [`Mixer::reserve`] made, a sender without a track keeps volume 1.
    pub fn set_volume(&mut self, session: u32, volume: f32) {
        let volume = if volume.is_nan() {
            1.0
        } else {
            volume.clamp(0.0, MAX_VOLUME)
        };
        if let Some(track) = self.tracks.iter_mut().find(|t| t.session == session) {
            track.volume = volume;
        }
        if let Some(entry) = self.volumes.iter_mut().find(|(s, _)| *s == session) {
            entry.1 = volume;
        } else if self.volumes.len() < self.volumes.capacity() {
            self.volumes.push((session, volume));
        }
    }

    fn volume_of(&self, session: u32) -> f32 {
        self.volumes
            .iter()
            .find(|(s, _)| *s == session)
            .map_or(1.0, |(_, v)| *v)
    }

    /// Makes a free track to hand to a running mixer. It allocates: call
    /// it off the playback thread.
    pub fn make_slot() -> Result<Slot, OpusError> {
        Ok(Slot(Box::new(Track::new(FREE)?)))
    }

    /// Adds a free track, which the next new sender takes. It does not
    /// allocate as long as [`Mixer::reserve`] made room for it.
    pub fn add_slot(&mut self, slot: Slot) {
        let mut track = slot.0;
        track.session = FREE;
        self.tracks.push(track);
    }

    /// The track of `session`: its own, else a free one, else a new one
    /// (which allocates; a call never gets there, see [`Mixer::make_slot`]).
    fn track(&mut self, session: u32) -> Result<&mut Track, OpusError> {
        let i = match self.tracks.iter().position(|t| t.session == session) {
            Some(i) => i,
            None => {
                let i = match self.tracks.iter().position(|t| t.session == FREE) {
                    Some(i) => {
                        self.tracks[i].reuse(session);
                        i
                    }
                    None => {
                        self.tracks.push(Box::new(Track::new(session)?));
                        self.tracks.len() - 1
                    }
                };
                // A new sender starts at their volume, with no glide.
                let volume = self.volume_of(session);
                self.tracks[i].volume = volume;
                self.tracks[i].gain = volume;
                i
            }
        };
        Ok(&mut self.tracks[i])
    }

    /// Handles what [`Inbound`](crate::inbound::Inbound) produced, at the
    /// time the packet arrived. The first frame of a sender without a track
    /// takes a free one, or makes one (see [`Mixer::add_track`]).
    pub fn push(&mut self, now_us: u64, event: Event) -> Result<(), OpusError> {
        match event {
            Event::Audio { session, arrival } => {
                let min_target_us = self.min_target_us;
                self.track(session)?.push(now_us, arrival, min_target_us);
            }
            Event::Restarted { session } => {
                if let Some(track) = self.tracks.iter_mut().find(|t| t.session == session) {
                    track.buffer.reset();
                }
            }
            Event::Left { session } => {
                if let Some(track) = self.tracks.iter_mut().find(|t| t.session == session) {
                    self.departed = add(self.departed, track.stats().jitter);
                    track.reuse(FREE);
                }
                // Sessions are never reused; forget the volume.
                self.volumes.retain(|(s, _)| *s != session);
            }
        }
        Ok(())
    }

    /// Writes the next `out.len()` samples of the mix. `now_us` is the time
    /// at which `out[0]` is due, on the clock the arrivals were stamped with.
    pub fn render(&mut self, now_us: u64, out: &mut [f32]) {
        out.fill(0.0);
        for track in &mut self.tracks {
            track.render(now_us, out);
        }
        for s in out.iter_mut() {
            *s = soft_clip(*s);
        }
    }

    /// The jitter counters of the senders who left, summed: with
    /// [`Mixer::stats`], everything this mixer played.
    pub fn departed(&self) -> JitterStats {
        self.departed
    }

    /// What each sender's track is doing, in the order the tracks were
    /// made; free tracks are left out.
    pub fn stats(&self) -> Vec<TrackStats> {
        self.track_stats().collect()
    }

    /// Like [`Mixer::stats`], without collecting them: the playback
    /// callback copies them into a list that already has room.
    pub fn track_stats(&self) -> impl Iterator<Item = TrackStats> + '_ {
        self.tracks
            .iter()
            .filter(|t| t.session != FREE)
            .map(|t| t.stats())
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
    use crate::jitter::FrameBytes;
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
                    primary: FrameBytes::new(&encoder.encode(&tone(f)).unwrap()).unwrap(),
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

    /// Plays one 100 ms talk spurt of sender 1 through `mixer`; returns
    /// the energy of the mix, and whether the track said it was speaking
    /// in the middle of the spurt and after it.
    fn play_a_spurt(mixer: &mut Mixer) -> (f32, bool, bool) {
        let frame = FrameSize::Ms5;
        let mut encoder = Encoder::new(frame, 64_000).unwrap();
        let tone: Vec<f32> = (0..frame.samples())
            .map(|i| 0.1 * (i as f32 * 0.05).sin())
            .collect();
        let mut energy = 0.0;
        let mut out = [0.0f32; 240];
        let (mut during, mut after) = (false, true);
        for k in 0..40u32 {
            let t = u64::from(k) * 5_000;
            if k < 20 {
                let arrival = Arrival {
                    timestamp: k * frame.samples() as u32,
                    end_of_spurt: k == 19,
                    primary: FrameBytes::new(&encoder.encode(&tone).unwrap()).unwrap(),
                    redundant: None,
                };
                mixer
                    .push(
                        t,
                        Event::Audio {
                            session: 1,
                            arrival,
                        },
                    )
                    .unwrap();
            }
            mixer.render(t, &mut out);
            energy += out.iter().map(|x| x * x).sum::<f32>();
            let speaking = mixer.stats()[0].speaking;
            match k {
                10 => during = speaking,
                39 => after = speaking,
                _ => {}
            }
        }
        (energy, during, after)
    }

    #[test]
    fn a_new_volume_glides_in_instead_of_clicking() {
        // A track in the middle of a frame of constant samples, turned up
        // from 100 % to 200 %.
        let mut track = Track::new(1).unwrap();
        track.frame = Some(FrameSize::Ms10);
        track.pcm.resize(FrameSize::Ms10.samples(), 0.5);
        track.volume = 2.0;
        let mut out = vec![0.0f32; 300];
        track.render(0, &mut out);
        // Out: 0.5 × the gain, which climbs 1/240 per sample to 2.
        let steps = out.windows(2).map(|w| w[1] - w[0]);
        assert!(
            steps
                .clone()
                .all(|d| (0.0..=0.5 * GAIN_STEP + 1e-6).contains(&d)),
            "{out:?}"
        );
        assert!(out[0] < 0.51, "no jump at the start: {}", out[0]);
        assert_eq!(out[299], 1.0, "there after 5 ms");
        assert_eq!(track.gain, 2.0);
    }

    #[test]
    fn each_sender_has_a_volume_and_a_speaking_light() {
        let mut plain = Mixer::new();
        let (energy, during, after) = play_a_spurt(&mut plain);
        assert!(energy > 0.1, "{energy}");
        assert!(during, "speaking while the spurt plays");
        assert!(!after, "silent once it ended");
        assert_eq!(plain.stats()[0].volume, 1.0);

        // A volume set before the sender's first frame waits for their
        // track. Twice the amplitude is four times the energy: the mix
        // stays under the soft clip's knee.
        let mut louder = Mixer::new();
        louder.reserve(4);
        louder.set_volume(1, 2.0);
        let (loud, ..) = play_a_spurt(&mut louder);
        assert!((loud / energy - 4.0).abs() < 0.01, "{loud} / {energy}");
        assert_eq!(louder.stats()[0].volume, 2.0);

        // Clamped to 0 through 200 %; 0 mutes the sender for this client.
        let mut muted = Mixer::new();
        muted.reserve(4);
        muted.set_volume(1, -3.0);
        assert_eq!(play_a_spurt(&mut muted).0, 0.0);
        muted.set_volume(1, 9.0);
        assert_eq!(muted.stats()[0].volume, MAX_VOLUME);
    }

    #[test]
    fn a_track_left_behind_serves_the_next_sender() {
        let frame = FrameSize::Ms5;
        let mut encoder = Encoder::new(frame, 64_000).unwrap();
        let mut audio = |session: u32, k: u32| Event::Audio {
            session,
            arrival: Arrival {
                timestamp: k * frame.samples() as u32,
                end_of_spurt: false,
                primary: FrameBytes::new(&encoder.encode(&vec![0.0; frame.samples()]).unwrap())
                    .unwrap(),
                redundant: None,
            },
        };
        let mut mixer = Mixer::new();
        mixer.add_track(1).unwrap();
        mixer.push(0, audio(1, 0)).unwrap();
        assert_eq!(mixer.stats()[0].jitter.received, 1);

        mixer.push(5_000, Event::Left { session: 1 }).unwrap();
        assert!(mixer.stats().is_empty(), "a free track shows no stats");
        assert_eq!(mixer.departed().received, 1, "but its counters are kept");
        mixer.push(10_000, audio(2, 0)).unwrap();
        assert_eq!(mixer.tracks.len(), 1, "the free track was reused");
        let stats = mixer.stats();
        assert_eq!((stats[0].session, stats[0].jitter.received), (2, 1));

        // A slot handed in from elsewhere is the next new sender's.
        mixer.reserve(1);
        mixer.add_slot(Mixer::make_slot().unwrap());
        mixer.push(15_000, audio(3, 0)).unwrap();
        assert_eq!(mixer.tracks.len(), 2);
        assert_eq!(mixer.stats().len(), 2);
    }
}
