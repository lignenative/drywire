//! Measurements an app runs on the sound card alone, outside any call
//! (spec section 11, "Latency lab in the app", and section 10, "Gate
//! calibration"):
//!
//! - [`Loopback`]: clicks played on the output and heard back by the
//!   microphone, the earphone held close to it. The delay between writing
//!   a click and capturing it is the device latency, input plus output,
//!   as the system really delivers it, a wireless headset's included. A
//!   click is heard when the microphone rises well above the background
//!   it captured just before, not above a fixed level: headset
//!   microphones are often so quiet that a click loud in the ear arrives
//!   faint.
//! - [`LevelRecorder`]: the level of every microphone frame for a few
//!   seconds, from which [`calibrate`](crate::gate::calibrate) picks the
//!   voice gate's threshold.
//! - [`LevelMeter`]: the loudest microphone frame since it was last read,
//!   for as long as it runs: the settings' level meter out of a call.
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

/// How far above the loudest sample of the background a click must rise
/// to be heard: four times, 12 dB.
const ABOVE_BACKGROUND: f32 = 4.0;

/// The quietest click that can be heard, in amplitude (−50 dBFS): below
/// it, a near-silent microphone's faintest noise would pass for one.
const QUIETEST_CLICK: f32 = 0.003;

/// After a click is heard or given up, how long its echo may still ring
/// before the microphone counts as background again.
const RING_US: u64 = 100_000;

/// A click not heard within this long is given up.
const MAX_DELAY_US: u64 = 400_000;

/// The most measurements kept.
pub const MAX_MEASUREMENTS: usize = 64;

/// No click is waiting to be heard.
const NONE: u64 = u64::MAX;

fn us_per_samples(samples: usize) -> u64 {
    samples as u64 * 1_000_000 / u64::from(SAMPLE_RATE)
}

/// The samples captured in `us` microseconds, rounded up.
fn samples_in_us(us: u64) -> usize {
    (us * u64::from(SAMPLE_RATE)).div_ceil(1_000_000) as usize
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

/// What the input callback remembers between buffers: the background it
/// has heard, and the threshold of the click it waits for.
struct Ear {
    /// The loudest sample of the background since the last click, if
    /// there was a quiet moment to measure it.
    background: Option<f32>,
    /// The background the threshold was set from: the last one measured.
    reference: f32,
    /// The click the threshold was set for.
    armed_for: u64,
    threshold: f32,
    /// Background is measured from this time on, once the last click's
    /// echo has died out.
    quiet_from_us: u64,
}

impl Ear {
    fn new() -> Ear {
        Ear {
            background: None,
            reference: 0.0,
            armed_for: NONE,
            threshold: QUIETEST_CLICK,
            quiet_from_us: 0,
        }
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
                play_clicks(&shared, &mut position, out, shared.now_us());
            }) as OutputCallback
        };
        let input = {
            let shared = Arc::clone(&shared);
            let mut ear = Ear::new();
            Box::new(move |samples: &[f32]| {
                hear_clicks(&shared, &mut ear, samples, shared.now_us());
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

/// The output callback's work: clicks every [`CLICK_EVERY`] samples,
/// silence between them. `position` counts the samples played so far;
/// `now` is when this buffer starts to play.
fn play_clicks(shared: &LoopbackShared, position: &mut usize, out: &mut [f32], now: u64) {
    for (j, sample) in out.iter_mut().enumerate() {
        let n = *position + j;
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
    *position += out.len();
}

/// The input callback's work: measures the background between clicks,
/// and records the delay of the click waiting to be heard, if this
/// buffer holds it. `now` is when its last sample was captured; the
/// others came before, one sample period apart.
fn hear_clicks(shared: &LoopbackShared, ear: &mut Ear, samples: &[f32], now: u64) {
    let emitted = shared.emitted_us.load(Ordering::Relaxed);
    if emitted == NONE {
        if now >= ear.quiet_from_us {
            let peak = samples.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            ear.background = Some(ear.background.unwrap_or(0.0).max(peak));
        }
        return;
    }
    if ear.armed_for != emitted {
        ear.armed_for = emitted;
        // No quiet moment since the last click: the background has not
        // changed as far as anyone can tell.
        if let Some(background) = ear.background.take() {
            ear.reference = background;
        }
        ear.threshold = (ear.reference * ABOVE_BACKGROUND).max(QUIETEST_CLICK);
    }
    // Only what was captured after the click was written can be its echo:
    // a knock or a word just before it would read as a delay of zero.
    let since = now.saturating_sub(emitted);
    let first = samples.len().saturating_sub(samples_in_us(since));
    let heard = samples[first..]
        .iter()
        .position(|x| x.abs() > ear.threshold)
        .map(|i| now.saturating_sub(us_per_samples(samples.len() - first - i)));
    match heard {
        Some(at) => {
            // Only the click still waiting: a new one written meanwhile
            // keeps its own time.
            ear.quiet_from_us = now + RING_US;
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
        None if since > MAX_DELAY_US => {
            let _ = shared.emitted_us.compare_exchange(
                emitted,
                NONE,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
        None => {}
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

/// The loudest 5 ms microphone frame since the last read, as a call's
/// [`take_input_peak_dbfs`](crate::call::CallControls::take_input_peak_dbfs)
/// gives it, but with no call: what a level meter shows out of a call. The
/// output plays silence.
pub struct LevelMeter(Arc<AtomicU32>);

impl LevelMeter {
    /// A meter and the callbacks to give the device.
    pub fn new() -> (LevelMeter, InputCallback, OutputCallback) {
        // The mean square of the loudest frame, as `f32` bits: for
        // positive floats, a larger value has larger bits, so `fetch_max`
        // keeps the loudest without a lock.
        let peak = Arc::new(AtomicU32::new(0));
        let input = {
            let peak = Arc::clone(&peak);
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
                    let power = mean_square(&frame);
                    if power.is_finite() {
                        peak.fetch_max(power.to_bits(), Ordering::Relaxed);
                    }
                }
            }) as InputCallback
        };
        let output = Box::new(|out: &mut [f32]| out.fill(0.0)) as OutputCallback;
        (LevelMeter(peak), input, output)
    }

    /// The loudest frame since the last call, in dBFS (`-inf` for silence,
    /// or when nothing was captured).
    pub fn take_peak_dbfs(&self) -> f32 {
        power_to_dbfs(f32::from_bits(self.0.swap(0, Ordering::Relaxed)))
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
        // The echo device captures what it played one period (2.5 ms)
        // earlier; four more periods on the way to the loopback make a
        // device of about 12.5 ms, as a real one is never instant. With
        // no latency at all, a late wake-up followed by an early one
        // would make the echo look older than its click, and the
        // loopback rightly ignores such an echo.
        let (test, mut hear, output) = Loopback::new();
        let mut late = std::collections::VecDeque::from(vec![vec![0.0f32; 120]; 4]);
        let input = Box::new(move |samples: &[f32]| {
            late.push_back(samples.to_vec());
            hear(&late.pop_front().expect("four buffers"));
        }) as InputCallback;
        let mut device = FileIo::new(Source::Echo, 120, None);
        device.start(input, output).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while test.delays_ms().len() < 3 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        device.stop();
        let delays = test.delays_ms();
        assert!(delays.len() >= 3, "{delays:?} of {} clicks", test.clicks());
        // Timed on a real clock, so only roughly: the exact delay is
        // checked on a simulated clock below. No lower bound: on a busy
        // shared CI machine the callbacks bunch up, and a 12.5 ms device
        // has measured as little as 0.3 ms.
        let median = test.median_ms().unwrap();
        assert!(median > 0.0 && median < 50.0, "{delays:?}");
    }

    /// A simulated device for a loopback test.
    #[derive(Clone, Copy)]
    struct Sim {
        /// How late the microphone hears the output, in samples.
        delay: usize,
        /// How loud the microphone hears it: 1 for full scale.
        gain: f32,
        /// The background's loudest sample, as uniform noise.
        noise: f32,
        /// A loud sample in the input just before each click is written.
        knock: bool,
    }

    const SIM: Sim = Sim {
        delay: 517,
        gain: 1.0,
        noise: 0.0,
        knock: false,
    };

    /// Runs a loopback test for `seconds` on `sim`, in 5 ms buffers, on a
    /// clock the test sets. Returns the delays measured, in µs, sorted.
    fn simulate(sim: Sim, seconds: usize) -> Vec<u64> {
        let (test, _, _) = Loopback::new();
        let shared = &test.0;
        let mut ear = Ear::new();
        let period = 240;
        let mut position = 0;
        let mut played = Vec::new();
        let mut out = vec![0.0f32; period];
        let mut seed = 1u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed as f32 / u32::MAX as f32 * 2.0 - 1.0) * sim.noise
        };
        for k in 0..seconds * 200 {
            let now = k as u64 * 5_000;
            play_clicks(shared, &mut position, &mut out, now);
            played.extend_from_slice(&out);
            // Captured between now - 5 ms and now: samples (k-1)·240 to
            // k·240 of the device's time, which heard what was played
            // `delay` samples earlier.
            let mut heard: Vec<f32> = (0..period)
                .map(|i| {
                    let t = (k * period + i).checked_sub(period + sim.delay);
                    t.map_or(0.0, |t| played[t]) * sim.gain + noise()
                })
                .collect();
            if sim.knock && k > 0 && (k * period).is_multiple_of(CLICK_EVERY) {
                // 1 ms before this buffer's click.
                heard[period - 48] = 0.9;
            }
            hear_clicks(shared, &mut ear, &heard, now);
        }
        let mut delays: Vec<u64> = (0..shared.count.load(Ordering::Acquire))
            .map(|i| shared.delays_us[i].load(Ordering::Relaxed))
            .collect();
        delays.sort();
        delays
    }

    /// The median of a [`simulate`]d test, which must have heard nearly
    /// every click.
    fn simulated_median_us(sim: Sim, seconds: usize) -> u64 {
        let delays = simulate(sim, seconds);
        assert!(delays.len() >= seconds * 2 - 2, "{delays:?}");
        delays[delays.len() / 2]
    }

    #[test]
    fn a_loopback_measures_the_delay_the_device_adds() {
        // 0, one buffer, and a delay that is not a whole number of buffers.
        for delay in [0, 240, 517, 4_800] {
            let expected = us_per_samples(delay);
            let median = simulated_median_us(Sim { delay, ..SIM }, 3);
            assert!(
                median.abs_diff(expected) <= 21,
                "{delay} samples: {median} µs, not {expected}"
            );
        }
    }

    #[test]
    fn a_sound_before_the_click_is_not_its_echo() {
        // Without the rule, the knock 1 ms before each click would be
        // taken for it, at a delay of 0.
        let median = simulated_median_us(Sim { knock: true, ..SIM }, 3);
        assert!(median.abs_diff(us_per_samples(517)) <= 21, "{median} µs");
    }

    #[test]
    fn a_faint_click_is_heard_over_a_quiet_background() {
        // A headset microphone that hears the click at −46 dBFS, far below
        // what a fixed threshold for loud clicks would wait for.
        let sim = Sim {
            gain: 0.01,
            noise: 0.000_3,
            ..SIM
        };
        let median = simulated_median_us(sim, 3);
        assert!(median.abs_diff(us_per_samples(517)) <= 21, "{median} µs");
    }

    #[test]
    fn a_noisy_background_is_not_taken_for_a_click() {
        // Loud noise between the clicks, still well under them.
        let sim = Sim {
            gain: 0.5,
            noise: 0.05,
            ..SIM
        };
        let median = simulated_median_us(sim, 3);
        assert!(median.abs_diff(us_per_samples(517)) <= 21, "{median} µs");
        // A click buried in the noise is not heard at all, rather than
        // measured wrong.
        let buried = Sim {
            gain: 0.02,
            noise: 0.05,
            ..SIM
        };
        assert_eq!(simulate(buried, 3), Vec::<u64>::new());
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

    #[test]
    fn a_meter_gives_the_loudest_frame_since_it_was_read() {
        let (meter, mut input, _) = LevelMeter::new();
        assert_eq!(meter.take_peak_dbfs(), f32::NEG_INFINITY, "nothing yet");
        input(&vec![0.5f32; LEVEL_FRAME / 2]);
        assert_eq!(meter.take_peak_dbfs(), f32::NEG_INFINITY, "half a frame");
        // The frame completes, then a quieter one follows.
        input(&vec![0.5f32; LEVEL_FRAME / 2]);
        input(&vec![0.1f32; LEVEL_FRAME]);
        assert!((meter.take_peak_dbfs() - -6.02).abs() < 0.01, "the loudest");
        assert_eq!(meter.take_peak_dbfs(), f32::NEG_INFINITY, "read once");
        input(&vec![0.1f32; LEVEL_FRAME]);
        assert!((meter.take_peak_dbfs() - -20.0).abs() < 0.01);
    }
}
