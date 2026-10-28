//! Measurements an app runs on the sound card alone, outside any call
//! (spec section 11, "Latency lab in the app", and section 10, "Gate
//! calibration"):
//!
//! - [`Loopback`]: clicks played on the output and heard back by the
//!   microphone, the earphone held against it. The delay between writing
//!   a click and capturing it is the device latency, input plus output,
//!   as the system really delivers it, a wireless headset's included.
//! - [`LevelRecorder`]: the level of every microphone frame for a few
//!   seconds, from which [`calibrate`](crate::gate::calibrate) picks the
//!   voice gate's threshold.
//!
//! Both hand out ordinary [`InputCallback`] and [`OutputCallback`]s for any
//! [`AudioIo`](crate::audio::AudioIo), and keep the rules of a call's
//! callbacks: they never wait, allocate or free (ADR 0017). Their results
//! are atomics, read from another thread.
//!
//! A loopback measurement times the two callbacks with the system clock,
//! so it is as precise as their scheduling, a millisecond or so: good
//! enough to compare a wired earphone with a wireless headset, which is
//! what it is for. It is shown as a measurement of this machine, not
//! the mouth-to-ear latency, which needs the physical method of section 11.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use crate::audio::{InputCallback, OutputCallback, SAMPLE_RATE};
use crate::gate::{mean_square, power_to_dbfs};

/// Samples between two clicks: half a second, more than any device
/// latency, so a click is never mistaken for the next one's echo.
pub const CLICK_EVERY: usize = SAMPLE_RATE as usize / 2;

/// How long a click lasts: 2 ms of a 2 kHz square wave, sharp at its
/// start.
const CLICK_LEN: usize = 96;
const CLICK_HALF_PERIOD: usize = 12;
const CLICK_AMPLITUDE: f32 = 0.5;

/// The level, in amplitude, at which the microphone is taken to hear the
/// click. An earphone held against it is far louder than this; a room's
/// noise far quieter.
pub const DETECT_THRESHOLD: f32 = 0.1;

/// A click not heard within this long is given up.
const MAX_DELAY_US: u64 = 400_000;

/// The most measurements kept.
pub const MAX_MEASUREMENTS: usize = 64;

/// No click is waiting to be heard.
const NONE: u64 = u64::MAX;

fn us_per_samples(samples: usize) -> u64 {
    samples as u64 * 1_000_000 / u64::from(SAMPLE_RATE)
}

struct LoopbackShared {
    started: Instant,
    /// When the click waiting to be heard was written, in microseconds
    /// since `started`, or [`NONE`].
    emitted_us: AtomicU64,
    delays_us: [AtomicU64; MAX_MEASUREMENTS],
    count: AtomicUsize,
    /// Clicks played, heard or not.
    clicks: AtomicUsize,
}

impl LoopbackShared {
    fn now_us(&self) -> u64 {
        self.started.elapsed().as_micros() as u64
    }
}

/// A running loopback test; read it while the device plays its callbacks.
pub struct Loopback(Arc<LoopbackShared>);

impl Loopback {
    /// A test, and the callbacks to give the device. The first click is
    /// played half a second in, once the device has settled.
    pub fn new() -> (Loopback, InputCallback, OutputCallback) {
        let shared = Arc::new(LoopbackShared {
            started: Instant::now(),
            emitted_us: AtomicU64::new(NONE),
            delays_us: std::array::from_fn(|_| AtomicU64::new(0)),
            count: AtomicUsize::new(0),
            clicks: AtomicUsize::new(0),
        });

        let output = {
            let shared = Arc::clone(&shared);
            let mut position = 0usize;
            Box::new(move |out: &mut [f32]| {
                let now = shared.now_us();
                for (j, sample) in out.iter_mut().enumerate() {
                    let n = position + j;
                    let phase = n % CLICK_EVERY;
                    *sample = if n >= CLICK_EVERY && phase < CLICK_LEN {
                        if phase == 0 {
                            // Written now, j samples into this buffer.
                            let at = now + us_per_samples(j);
                            shared.emitted_us.store(at, Ordering::Relaxed);
                            shared.clicks.fetch_add(1, Ordering::Relaxed);
                        }
                        if (phase / CLICK_HALF_PERIOD).is_multiple_of(2) {
                            CLICK_AMPLITUDE
                        } else {
                            -CLICK_AMPLITUDE
                        }
                    } else {
                        0.0
                    };
                }
                position += out.len();
            }) as OutputCallback
        };

        let input = {
            let shared = Arc::clone(&shared);
            Box::new(move |samples: &[f32]| {
                let emitted = shared.emitted_us.load(Ordering::Relaxed);
                if emitted == NONE {
                    return;
                }
                let now = shared.now_us();
                // The last sample was captured just now; the others before.
                let heard = samples
                    .iter()
                    .position(|x| x.abs() > DETECT_THRESHOLD)
                    .map(|i| now.saturating_sub(us_per_samples(samples.len() - i)));
                match heard {
                    Some(at) => {
                        // Only the click still waiting: a new one written
                        // meanwhile keeps its own time.
                        if shared
                            .emitted_us
                            .compare_exchange(emitted, NONE, Ordering::Relaxed, Ordering::Relaxed)
                            .is_ok()
                        {
                            let delay = at.saturating_sub(emitted);
                            let i = shared.count.load(Ordering::Relaxed);
                            if delay <= MAX_DELAY_US && i < MAX_MEASUREMENTS {
                                shared.delays_us[i].store(delay, Ordering::Relaxed);
                                shared.count.store(i + 1, Ordering::Release);
                            }
                        }
                    }
                    None if now.saturating_sub(emitted) > MAX_DELAY_US => {
                        let _ = shared.emitted_us.compare_exchange(
                            emitted,
                            NONE,
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                        );
                    }
                    None => {}
                }
            }) as InputCallback
        };
        (Loopback(shared), input, output)
    }

    /// Every delay measured so far, in milliseconds.
    pub fn delays_ms(&self) -> Vec<f64> {
        let n = self.0.count.load(Ordering::Acquire);
        self.0.delays_us[..n]
            .iter()
            .map(|d| d.load(Ordering::Relaxed) as f64 / 1000.0)
            .collect()
    }

    /// How many clicks were played, heard or not.
    pub fn clicks(&self) -> usize {
        self.0.clicks.load(Ordering::Relaxed)
    }

    /// The median delay, in milliseconds, once one was measured.
    pub fn median_ms(&self) -> Option<f64> {
        let mut delays = self.delays_ms();
        if delays.is_empty() {
            return None;
        }
        delays.sort_by(f64::total_cmp);
        Some(delays[delays.len() / 2])
    }
}

/// The frames a [`LevelRecorder`] measures: 5 ms.
pub const LEVEL_FRAME: usize = SAMPLE_RATE as usize / 200;

struct LevelShared {
    /// Each frame's level in dBFS, as `f32` bits.
    levels: Box<[AtomicU32]>,
    count: AtomicUsize,
}

/// Records the level of every 5 ms microphone frame, up to a fixed number
/// of frames; the output plays silence.
pub struct LevelRecorder(Arc<LevelShared>);

impl LevelRecorder {
    /// A recorder with room for `frames` frames (made now, not in the
    /// callback), and the callbacks to give the device.
    pub fn new(frames: usize) -> (LevelRecorder, InputCallback, OutputCallback) {
        let shared = Arc::new(LevelShared {
            levels: (0..frames).map(|_| AtomicU32::new(0)).collect(),
            count: AtomicUsize::new(0),
        });
        let input = {
            let shared = Arc::clone(&shared);
            let mut frame = [0.0f32; LEVEL_FRAME];
            let mut filled = 0;
            Box::new(move |samples: &[f32]| {
                for &x in samples {
                    frame[filled] = x;
                    filled += 1;
                    if filled < LEVEL_FRAME {
                        continue;
                    }
                    filled = 0;
                    let i = shared.count.load(Ordering::Relaxed);
                    if i < shared.levels.len() {
                        let level = power_to_dbfs(mean_square(&frame));
                        shared.levels[i].store(level.to_bits(), Ordering::Relaxed);
                        shared.count.store(i + 1, Ordering::Release);
                    }
                }
            }) as InputCallback
        };
        let output = Box::new(|out: &mut [f32]| out.fill(0.0)) as OutputCallback;
        (LevelRecorder(shared), input, output)
    }

    /// The levels recorded so far, in dBFS.
    pub fn levels_dbfs(&self) -> Vec<f32> {
        let n = self.0.count.load(Ordering::Acquire);
        self.0.levels[..n]
            .iter()
            .map(|l| f32::from_bits(l.load(Ordering::Relaxed)))
            .collect()
    }

    /// The loudest of the last `frames` frames, in dBFS, for a live meter.
    pub fn recent_dbfs(&self, frames: usize) -> f32 {
        let levels = self.levels_dbfs();
        levels[levels.len().saturating_sub(frames)..]
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max)
    }

    /// Whether every frame it has room for is recorded.
    pub fn is_full(&self) -> bool {
        self.0.count.load(Ordering::Acquire) >= self.0.levels.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioIo;
    use crate::audio::file::{FileIo, Source};
    use std::time::Duration;

    #[test]
    fn a_loopback_hears_its_clicks_back() {
        // The echo device captures what it played one period earlier: a
        // device with almost no latency of its own.
        let (test, input, output) = Loopback::new();
        let mut device = FileIo::new(Source::Echo, 120, None);
        device.start(input, output).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while test.delays_ms().len() < 3 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        device.stop();
        let delays = test.delays_ms();
        assert!(delays.len() >= 3, "{delays:?} of {} clicks", test.clicks());
        let median = test.median_ms().unwrap();
        assert!(median < 50.0, "{delays:?}");
    }

    #[test]
    fn a_silent_microphone_hears_nothing() {
        let (test, input, mut output) = Loopback::new();
        let mut input = input;
        let mut out = vec![0.0; 480];
        let silence = vec![0.0; 480];
        for _ in 0..200 {
            output(&mut out);
            input(&silence);
        }
        assert!(test.clicks() >= 1);
        assert!(test.delays_ms().is_empty());
        assert!(test.median_ms().is_none());
    }

    #[test]
    fn levels_are_recorded_frame_by_frame() {
        let (recorder, mut input, _) = LevelRecorder::new(4);
        let loud = vec![0.5f32; LEVEL_FRAME * 3 / 2];
        input(&loud); // one frame and a half
        assert_eq!(recorder.levels_dbfs().len(), 1);
        input(&vec![0.0f32; LEVEL_FRAME * 4]);
        let levels = recorder.levels_dbfs();
        assert_eq!(levels.len(), 4, "full: the rest is dropped");
        assert!((levels[0] - -6.02).abs() < 0.01, "{levels:?}");
        assert!(recorder.is_full());
        // The second frame was half loud: 3 dB quieter.
        assert!((levels[1] - -9.03).abs() < 0.01, "{levels:?}");
        assert_eq!(recorder.recent_dbfs(4), levels[0]);
        assert_eq!(recorder.recent_dbfs(2), f32::NEG_INFINITY);
    }
}
