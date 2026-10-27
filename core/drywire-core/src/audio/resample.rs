//! A small resampler, for microphones that do not run at 48 kHz.
//!
//! Core Audio never converts an input's sample rate (ADR 0013), and many
//! USB microphones run at 44.1 kHz. Rather than refuse them, the backend
//! captures at the device's rate and converts here, in its callback
//! (ADR 0018).
//!
//! It is a **polyphase windowed-sinc** resampler. The ratio between the two
//! rates is reduced to `up / down` (160 / 147 from 44.1 kHz), and output
//! sample `n` sits at input position `n × down / up`: its integer part says
//! which input samples to use, its fraction which of `up` precomputed
//! filters, so no sine is computed in the callback. Each filter is a
//! sinc low-pass, cut just below the lower of the two Nyquist
//! frequencies, shaped by a Kaiser window and normalised to a gain of 1.
//!
//! **Latency** is half the filter: 16 input samples at 44.1 kHz with the
//! 32-tap filters, 0.36 ms ([`Resampler::delay_us`]). That is the price of
//! a filter steep enough to keep the voice band flat to about 17 kHz and
//! push images and aliases about 60 dB down. A linear interpolation would
//! add no delay but dull the voice and alias audibly; a longer filter
//! would add delay for no audible gain.
//!
//! Like everything in an audio callback, [`Resampler::process`] never
//! allocates (ADR 0017): its tables and history are made in
//! [`Resampler::new`].

use std::f64::consts::PI;

/// Taps of each filter when the rate goes up; more when it goes down, so
/// the transition stays as narrow at the output rate.
const TAPS: usize = 32;

/// How far down images and aliases go, in dB. Kaiser's formulas turn it
/// into the window's shape and the width of the transition band.
const ATTENUATION_DB: f64 = 60.0;

/// The Kaiser window's shape for [`ATTENUATION_DB`] (Kaiser's formula for
/// more than 50 dB).
const KAISER_BETA: f64 = 0.1102 * (ATTENUATION_DB - 8.7);

/// A ratio needing more filters than this is not a real device rate.
const MAX_PHASES: usize = 1024;

/// Converts a stream of samples from one rate to another.
pub struct Resampler {
    up: u64,
    down: u64,
    taps: usize,
    /// `up` filters of `taps` coefficients each, filter `p` for output
    /// samples at a fraction `p / up` past an input sample.
    filters: Vec<f32>,
    /// The last `taps` input samples, twice, so that they can be read as
    /// one slice whatever the write position: `history[at..at + taps]`.
    history: Vec<f32>,
    at: usize,
    /// Input samples received, and output samples produced.
    received: u64,
    produced: u64,
}

impl Resampler {
    /// A resampler from `from` Hz to `to` Hz, or `None` if the ratio of
    /// the two is not one a real device has (it would need more than
    /// [`MAX_PHASES`] filters).
    pub fn new(from: u32, to: u32) -> Option<Resampler> {
        if from == 0 || to == 0 {
            return None;
        }
        let g = gcd(u64::from(from), u64::from(to));
        let (up, down) = (u64::from(to) / g, u64::from(from) / g);
        if up as usize > MAX_PHASES {
            return None;
        }
        // Going down, the filter must cut below the output's Nyquist
        // frequency: as many more taps as the ratio, for the same steepness.
        let taps = TAPS * (down.div_ceil(up) as usize).max(1);
        // Cut-off in cycles per input sample: half a transition band below
        // half the lower rate, so the stop band starts where images and
        // aliases would.
        let nyquist = 0.5 * (up as f64 / down as f64).min(1.0);
        let transition = (ATTENUATION_DB - 8.0) / (2.285 * 2.0 * PI * taps as f64);
        let cutoff = nyquist - transition / 2.0;
        let half = taps as f64 / 2.0;

        let mut filters = Vec::with_capacity(up as usize * taps);
        for p in 0..up {
            let fraction = p as f64 / up as f64;
            let start = filters.len();
            for k in 0..taps {
                // Distance from the output position to input sample k of
                // the window, oldest first: the window spans the `taps`
                // samples around the position, half before, half after.
                let x = fraction + half - 1.0 - k as f64;
                filters.push((sinc(2.0 * cutoff * x) * kaiser(x / half)) as f32);
            }
            // A gain of exactly 1 for every phase: no ripple at the rate of
            // the phases, and silence stays silence.
            let sum: f32 = filters[start..].iter().sum();
            for c in &mut filters[start..] {
                *c /= sum;
            }
        }
        Some(Resampler {
            up,
            down,
            taps,
            filters,
            history: vec![0.0; 2 * taps],
            at: 0,
            received: 0,
            produced: 0,
        })
    }

    /// The most output samples `input` samples can produce, to size the
    /// output buffer before the stream starts.
    pub fn max_output(&self, input: usize) -> usize {
        (input as u64 * self.up).div_ceil(self.down) as usize + 1
    }

    /// The delay the filter adds, in microseconds: half its taps, at the
    /// input rate.
    pub fn delay_us(&self, from: u32) -> f64 {
        (self.taps / 2) as f64 * 1e6 / f64::from(from)
    }

    /// Converts `input` and writes the samples it completes to `output`,
    /// returning how many. `output` must hold
    /// [`Resampler::max_output`]`(input.len())`, which the backend sizes
    /// once from the largest buffer the OS may hand over. A shorter `output`
    /// would corrupt the stream: the samples that did not fit would be
    /// computed later, from the wrong window.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> usize {
        let half = (self.taps / 2) as u64;
        let mut written = 0;
        for &x in input {
            self.history[self.at] = x;
            self.history[self.at + self.taps] = x;
            self.at = (self.at + 1) % self.taps;
            let newest = self.received;
            self.received += 1;
            // Every output whose window now has all its samples: the one
            // at input position `i + fraction` needs up to sample `i + half`.
            loop {
                let position = self.produced * self.down;
                let i = position / self.up;
                if i + half > newest || written == output.len() {
                    break;
                }
                let phase = (position % self.up) as usize;
                let filter = &self.filters[phase * self.taps..(phase + 1) * self.taps];
                let window = &self.history[self.at..self.at + self.taps];
                output[written] = filter.iter().zip(window).map(|(c, s)| c * s).sum();
                written += 1;
                self.produced += 1;
            }
        }
        written
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

/// The Kaiser window at `x` in `[-1, 1]`, zero outside.
fn kaiser(x: f64) -> f64 {
    if x.abs() > 1.0 {
        return 0.0;
    }
    bessel_i0(KAISER_BETA * (1.0 - x * x).sqrt()) / bessel_i0(KAISER_BETA)
}

/// The modified Bessel function of the first kind, order 0, by its series;
/// 30 terms are far more than enough for the window's arguments.
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    for k in 1..30 {
        let r = x / (2.0 * k as f64);
        term *= r * r;
        sum += term;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, hz: f64, seconds: f64) -> Vec<f32> {
        let n = (f64::from(rate) * seconds) as usize;
        (0..n)
            .map(|i| (0.5 * (2.0 * PI * hz * i as f64 / f64::from(rate)).sin()) as f32)
            .collect()
    }

    fn resample(from: u32, to: u32, input: &[f32]) -> Vec<f32> {
        let mut r = Resampler::new(from, to).unwrap();
        let mut out = vec![0.0; r.max_output(input.len())];
        let n = r.process(input, &mut out);
        out.truncate(n);
        out
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
    }

    #[test]
    fn one_second_in_is_one_second_out_minus_the_delay() {
        let r = Resampler::new(44_100, 48_000).unwrap();
        assert_eq!((r.up, r.down), (160, 147));
        assert!(
            (r.delay_us(44_100) - 362.8).abs() < 0.1,
            "16 samples at 44.1 kHz"
        );
        let out = resample(44_100, 48_000, &vec![0.0; 44_100]);
        // Each output waits for the 16 input samples after it, so the last
        // 16 input samples' outputs are still waiting: 16 × 48 / 44.1,
        // about 17. That wait is the latency.
        assert!((47_980..=47_985).contains(&out.len()), "{}", out.len());
    }

    #[test]
    fn a_voice_band_tone_comes_out_the_same() {
        for (from, hz) in [
            (44_100, 1_000.0),
            (44_100, 7_000.0),
            (32_000, 3_000.0),
            (96_000, 5_000.0),
        ] {
            let input = tone(from, hz, 0.5);
            let out = resample(from, 48_000, &input);
            // Output sample n is the input signal at n / 48 kHz: the delay
            // is in when it comes out, not in what it holds.
            let expected: Vec<f32> = (0..out.len())
                .map(|n| (0.5 * (2.0 * PI * hz * n as f64 / 48_000.0).sin()) as f32)
                .collect();
            // Skip the first 10 ms, where the filter fills.
            let skip = 480;
            let error: Vec<f32> = out[skip..]
                .iter()
                .zip(&expected[skip..])
                .map(|(a, b)| a - b)
                .collect();
            let snr = 20.0 * (rms(&expected[skip..]) / rms(&error)).log10();
            assert!(snr > 55.0, "{from} Hz, {hz} Hz tone: SNR {snr:.1} dB");
        }
    }

    #[test]
    fn going_down_removes_what_the_output_cannot_hold() {
        // 30 kHz fits at 96 kHz but not at 48 kHz, where it would alias
        // to 18 kHz.
        let out = resample(96_000, 48_000, &tone(96_000, 30_000.0, 0.2));
        let level = 20.0 * (rms(&out[480..]) / (0.5 / 2f64.sqrt())).log10();
        assert!(level < -50.0, "alias at {level:.1} dB");
    }

    #[test]
    fn chunks_of_any_size_give_the_same_samples() {
        let input = tone(44_100, 440.0, 0.1);
        let whole = resample(44_100, 48_000, &input);
        let mut r = Resampler::new(44_100, 48_000).unwrap();
        let mut out = vec![0.0; r.max_output(512)];
        let mut pieces = Vec::new();
        let mut rest = &input[..];
        for size in [1, 15, 512, 7, 100].iter().cycle() {
            if rest.is_empty() {
                break;
            }
            let (chunk, more) = rest.split_at((*size).min(rest.len()));
            let n = r.process(chunk, &mut out);
            pieces.extend_from_slice(&out[..n]);
            rest = more;
        }
        assert_eq!(pieces, whole);
    }

    #[test]
    fn silence_stays_silence_and_a_constant_keeps_its_level() {
        assert!(
            resample(44_100, 48_000, &[0.0; 1000])
                .iter()
                .all(|s| *s == 0.0)
        );
        let out = resample(44_100, 48_000, &[0.25; 4410]);
        for s in &out[100..] {
            assert!((s - 0.25).abs() < 1e-5, "{s}");
        }
    }

    #[test]
    fn rates_no_device_has_are_refused() {
        assert!(Resampler::new(0, 48_000).is_none());
        assert!(Resampler::new(44_101, 48_000).is_none());
        assert!(Resampler::new(48_000, 48_000).is_some());
    }
}
