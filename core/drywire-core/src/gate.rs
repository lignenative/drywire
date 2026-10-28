//! The voice gate (spec section 7): decides, frame by frame, whether the
//! microphone's frame is sent. It never changes a sample: it only reads them.
//!
//! - **Instant open.** The first frame above the open threshold is sent, with
//!   no lookahead, so the gate adds no latency.
//! - **Hold.** After the level drops, the gate stays open for a while, so the
//!   ends of words and short pauses are not cut.
//! - **Hysteresis.** It closes at a lower threshold than it opens, so a level
//!   hovering around one threshold does not flap.
//!
//! The last frame of a talk spurt is the one on which the hold runs out. It is
//! still sent, flagged `end_of_spurt`: knowing it is the last needs no
//! lookahead, and one more frame of near-silence costs nothing.

use crate::opus::{FrameSize, SAMPLE_RATE};

/// Thresholds and hold of a [`VoiceGate`]. The thresholds are calibrated per
/// user; the defaults are a starting point, not a measurement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GateConfig {
    /// Level, in dBFS, at or above which a closed gate opens.
    pub open_dbfs: f32,
    /// Level, in dBFS, at or above which an open gate restarts its hold.
    /// Must not be above `open_dbfs`.
    pub close_dbfs: f32,
    /// How long the gate stays open after the level falls below
    /// `close_dbfs`, in milliseconds.
    pub hold_ms: u32,
}

impl Default for GateConfig {
    fn default() -> GateConfig {
        GateConfig {
            open_dbfs: -40.0,
            close_dbfs: -46.0,
            hold_ms: 150,
        }
    }
}

/// What to do with one microphone frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateDecision {
    /// The gate is closed: send nothing.
    Silent,
    /// Encode and send this frame.
    Send {
        /// First frame of a talk spurt: the sender repeats its key
        /// announcement just before it (protocol section 8.3).
        start_of_spurt: bool,
        /// Last frame of the talk spurt: set the `end_of_spurt` flag.
        end_of_spurt: bool,
    },
}

/// The gate's state for one microphone.
#[derive(Clone, Debug)]
pub struct VoiceGate {
    config: GateConfig,
    frame: FrameSize,
    hold_frames: u32,
    /// Frames left before closing; 0 means the gate is closed.
    remaining: u32,
}

impl VoiceGate {
    /// A closed gate for frames of `frame` size.
    ///
    /// # Panics
    ///
    /// If `close_dbfs` is above `open_dbfs`, which would make the gate close
    /// while the level is still above its open threshold.
    pub fn new(config: GateConfig, frame: FrameSize) -> VoiceGate {
        assert!(
            config.close_dbfs <= config.open_dbfs,
            "the close threshold must not be above the open threshold"
        );
        VoiceGate {
            config,
            frame,
            hold_frames: hold_frames(config.hold_ms, frame),
            remaining: 0,
        }
    }

    /// The thresholds and hold in use.
    pub fn config(&self) -> GateConfig {
        self.config
    }

    /// Changes the thresholds and hold, from the next frame on. A close
    /// threshold above the open one is lowered to it, rather than refused:
    /// this runs in the capture callback, where nothing may panic. An open
    /// gate stays open, with at most the new hold left.
    pub fn set_config(&mut self, config: GateConfig) {
        self.config = GateConfig {
            close_dbfs: config.close_dbfs.min(config.open_dbfs),
            ..config
        };
        self.hold_frames = hold_frames(config.hold_ms, self.frame);
        self.remaining = self.remaining.min(self.hold_frames);
    }

    /// Whether the gate is open.
    pub fn is_open(&self) -> bool {
        self.remaining > 0
    }

    /// Decides what to do with the next frame.
    pub fn process(&mut self, frame: &[f32]) -> GateDecision {
        let level = level_dbfs(frame);
        if !self.is_open() {
            if level < self.config.open_dbfs {
                return GateDecision::Silent;
            }
            self.remaining = self.hold_frames;
            return GateDecision::Send {
                start_of_spurt: true,
                end_of_spurt: false,
            };
        }
        if level >= self.config.close_dbfs {
            self.remaining = self.hold_frames;
        } else {
            self.remaining -= 1;
        }
        GateDecision::Send {
            start_of_spurt: false,
            end_of_spurt: self.remaining == 0,
        }
    }
}

/// How many frames a hold of `hold_ms` lasts: at least one, so the frame
/// that opens the gate is not also its last.
fn hold_frames(hold_ms: u32, frame: FrameSize) -> u32 {
    let frame_ms = frame.samples() as f64 * 1000.0 / f64::from(SAMPLE_RATE);
    (f64::from(hold_ms) / frame_ms).ceil().max(1.0) as u32
}

/// The mean of the squared samples: the frame's power, 1 for a full-scale
/// square wave, 0 for digital silence.
pub fn mean_square(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    frame.iter().map(|x| x * x).sum::<f32>() / frame.len() as f32
}

/// A [`mean_square`] in dBFS: `-inf` for silence.
pub fn power_to_dbfs(mean_square: f32) -> f32 {
    10.0 * mean_square.log10()
}

/// The RMS level of a frame in dBFS: 0 for a full-scale square wave, about
/// −3 for a full-scale sine, `-inf` for digital silence.
pub fn level_dbfs(frame: &[f32]) -> f32 {
    power_to_dbfs(mean_square(frame))
}

/// How far above the background noise's loudest moments the calibrated
/// gate opens.
pub const CALIBRATION_MARGIN_DB: f32 = 4.0;

/// The range a calibrated threshold stays in: below −70 dBFS no
/// microphone is that quiet, above −20 dBFS speech may not open the gate.
pub const CALIBRATION_RANGE_DBFS: (f32, f32) = (-70.0, -20.0);

/// What [`calibrate`] found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calibration {
    /// The background noise's typical level (the median frame), in dBFS.
    pub noise_dbfs: f32,
    /// Its loudest moments (the 99th percentile frame): controller
    /// clicks, keys, the game leaking from a headset.
    pub peak_dbfs: f32,
    /// The gate to use: it opens [`CALIBRATION_MARGIN_DB`] above the
    /// peaks, and closes as far below that as the default gate does.
    pub gate: GateConfig,
    /// Whether the noise is so loud that the threshold hit the top of its
    /// range: quiet speech may not open the gate. The person should lower
    /// the game or move the microphone.
    pub too_loud: bool,
}

/// The gate for a microphone whose background noise, recorded frame by
/// frame for a while with the person silent but playing, had these levels
/// in dBFS (spec section 10, "Gate calibration"). `None` without levels.
pub fn calibrate(levels_dbfs: &[f32]) -> Option<Calibration> {
    if levels_dbfs.is_empty() {
        return None;
    }
    // Digital silence is -inf: count it as far below anything real.
    let mut levels: Vec<f32> = levels_dbfs.iter().map(|l| l.max(-120.0)).collect();
    levels.sort_by(f32::total_cmp);
    let at = |q: f64| levels[((levels.len() - 1) as f64 * q).round() as usize];
    let (noise_dbfs, peak_dbfs) = (at(0.5), at(0.99));
    let wanted = peak_dbfs + CALIBRATION_MARGIN_DB;
    let (low, high) = CALIBRATION_RANGE_DBFS;
    let open_dbfs = wanted.clamp(low, high);
    let defaults = GateConfig::default();
    Some(Calibration {
        noise_dbfs,
        peak_dbfs,
        gate: GateConfig {
            open_dbfs,
            close_dbfs: open_dbfs - (defaults.open_dbfs - defaults.close_dbfs),
            hold_ms: defaults.hold_ms,
        },
        too_loud: wanted > high,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: FrameSize = FrameSize::Ms5;

    /// A frame whose RMS level is `dbfs`: a constant of that amplitude.
    fn frame_at(dbfs: f32) -> Vec<f32> {
        vec![10f32.powf(dbfs / 20.0); FRAME.samples()]
    }

    fn send(start: bool, end: bool) -> GateDecision {
        GateDecision::Send {
            start_of_spurt: start,
            end_of_spurt: end,
        }
    }

    #[test]
    fn level_of_known_signals() {
        assert!((level_dbfs(&frame_at(-20.0)) - -20.0).abs() < 0.01);
        assert_eq!(level_dbfs(&[0.0; 4]), f32::NEG_INFINITY);
    }

    #[test]
    fn opens_on_the_first_loud_frame() {
        let mut gate = VoiceGate::new(GateConfig::default(), FRAME);
        assert_eq!(gate.process(&frame_at(-60.0)), GateDecision::Silent);
        assert_eq!(gate.process(&frame_at(-30.0)), send(true, false));
        assert!(gate.is_open());
    }

    #[test]
    fn holds_then_closes_with_an_end_of_spurt_frame() {
        let config = GateConfig::default(); // 150 ms hold: 30 frames of 5 ms
        let mut gate = VoiceGate::new(config, FRAME);
        gate.process(&frame_at(-30.0));

        let quiet = frame_at(-70.0);
        for i in 0..29 {
            assert_eq!(gate.process(&quiet), send(false, false), "hold frame {i}");
        }
        assert_eq!(gate.process(&quiet), send(false, true));
        assert_eq!(gate.process(&quiet), GateDecision::Silent);
    }

    #[test]
    fn hysteresis_keeps_an_open_gate_open_between_thresholds() {
        let mut gate = VoiceGate::new(GateConfig::default(), FRAME);
        let between = frame_at(-43.0); // under open (−40), over close (−46)

        // Closed: a level between the thresholds does not open it.
        assert_eq!(gate.process(&between), GateDecision::Silent);

        // Open: the same level keeps restarting the hold.
        gate.process(&frame_at(-30.0));
        for _ in 0..1000 {
            assert_eq!(gate.process(&between), send(false, false));
        }
    }

    #[test]
    fn a_loud_frame_during_the_hold_restarts_it() {
        let mut gate = VoiceGate::new(GateConfig::default(), FRAME);
        gate.process(&frame_at(-30.0));
        for _ in 0..20 {
            gate.process(&frame_at(-70.0));
        }
        gate.process(&frame_at(-30.0));
        for _ in 0..29 {
            assert_eq!(gate.process(&frame_at(-70.0)), send(false, false));
        }
        assert_eq!(gate.process(&frame_at(-70.0)), send(false, true));
    }

    #[test]
    fn thresholds_change_while_the_gate_runs() {
        let mut gate = VoiceGate::new(GateConfig::default(), FRAME);
        let speech = frame_at(-30.0);
        assert_eq!(gate.process(&speech), send(true, false));
        // A threshold raised above the speech: the hold runs out, at most
        // the new hold (5 ms here, one frame) later.
        gate.set_config(GateConfig {
            open_dbfs: -20.0,
            close_dbfs: -26.0,
            hold_ms: 5,
        });
        assert_eq!(gate.process(&speech), send(false, true));
        assert_eq!(gate.process(&speech), GateDecision::Silent);
        // Lowered again, it opens on the same level.
        gate.set_config(GateConfig::default());
        assert_eq!(gate.process(&speech), send(true, false));
    }

    #[test]
    fn a_close_threshold_above_the_open_one_is_lowered() {
        let mut gate = VoiceGate::new(GateConfig::default(), FRAME);
        gate.set_config(GateConfig {
            open_dbfs: -50.0,
            close_dbfs: -10.0,
            hold_ms: 150,
        });
        assert_eq!(gate.config().close_dbfs, -50.0);
    }

    #[test]
    fn calibration_opens_just_above_the_noise_and_its_clicks() {
        // Two seconds of 5 ms frames: noise at −60 dBFS, and 2 % of
        // controller clicks at −45.
        let mut levels = vec![-60.0; 400];
        for i in (0..400).step_by(50) {
            levels[i] = -45.0;
        }
        let c = calibrate(&levels).unwrap();
        assert_eq!(c.noise_dbfs, -60.0);
        assert_eq!(c.peak_dbfs, -45.0);
        assert_eq!(c.gate.open_dbfs, -41.0);
        assert_eq!(c.gate.close_dbfs, -47.0);
        assert!(!c.too_loud);

        // A silent microphone, and a very loud room.
        let quiet = calibrate(&[f32::NEG_INFINITY; 100]).unwrap();
        assert_eq!(quiet.gate.open_dbfs, CALIBRATION_RANGE_DBFS.0);
        let loud = calibrate(&[-15.0; 100]).unwrap();
        assert_eq!(loud.gate.open_dbfs, CALIBRATION_RANGE_DBFS.1);
        assert!(loud.too_loud);
        assert!(calibrate(&[]).is_none());
    }

    #[test]
    fn hold_rounds_up_to_whole_frames() {
        let config = GateConfig {
            hold_ms: 6,
            ..GateConfig::default()
        };
        // 6 ms of 2.5 ms frames is 2.4 frames: hold for 3.
        assert_eq!(VoiceGate::new(config, FrameSize::Ms2_5).hold_frames, 3);
        let none = GateConfig {
            hold_ms: 0,
            ..GateConfig::default()
        };
        assert_eq!(VoiceGate::new(none, FrameSize::Ms10).hold_frames, 1);
    }
}
