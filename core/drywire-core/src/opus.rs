//! A small safe wrapper around libopus, configured the one way Drywire uses
//! it: 48 kHz, mono, `OPUS_APPLICATION_RESTRICTED_LOWDELAY` (CELT only, the
//! lowest algorithmic delay Opus offers), frames of 2.5, 5 or 10 ms.
//!
//! libopus is C; it is built from source by the `opusic-sys` crate (ADR
//! 0009). Every `unsafe` block below is a call into it, and each says why the
//! call is sound.

use std::ffi::{CStr, c_int};
use std::fmt;
use std::ptr::NonNull;

use opusic_sys as ffi;

/// Drywire always runs Opus at 48 kHz.
pub const SAMPLE_RATE: u32 = 48_000;

/// The largest encoded frame we accept or produce. Two of them, plus the
/// header, the redundant length and the tag, still fit in one 1200-byte
/// datagram: `16 + 2 + 2 × 575 + 16 = 1184`. At 64 kb/s a 10 ms frame is
/// about 80 bytes, so this is never a constraint in practice.
pub const MAX_FRAME_BYTES: usize = 575;

/// The frame sizes Drywire supports (spec section 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameSize {
    /// 2.5 ms, 120 samples.
    Ms2_5,
    /// 5 ms, 240 samples.
    Ms5,
    /// 10 ms, 480 samples.
    Ms10,
}

impl FrameSize {
    /// Every supported size, smallest first.
    pub const ALL: [FrameSize; 3] = [FrameSize::Ms2_5, FrameSize::Ms5, FrameSize::Ms10];

    /// Samples per frame at 48 kHz.
    pub fn samples(self) -> usize {
        match self {
            FrameSize::Ms2_5 => 120,
            FrameSize::Ms5 => 240,
            FrameSize::Ms10 => 480,
        }
    }

    /// Duration of a frame in microseconds.
    pub fn micros(self) -> u64 {
        self.samples() as u64 * 1_000_000 / u64::from(SAMPLE_RATE)
    }

    /// The size whose frames hold `samples` samples, if Drywire supports it.
    pub fn from_samples(samples: usize) -> Option<FrameSize> {
        FrameSize::ALL.into_iter().find(|f| f.samples() == samples)
    }
}

impl fmt::Display for FrameSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FrameSize::Ms2_5 => "2.5 ms",
            FrameSize::Ms5 => "5 ms",
            FrameSize::Ms10 => "10 ms",
        })
    }
}

/// An error code returned by libopus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpusError(c_int);

impl fmt::Display for OpusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // SAFETY: opus_strerror accepts any code and returns a pointer to a
        // static, NUL-terminated string.
        let message = unsafe { CStr::from_ptr(ffi::opus_strerror(self.0)) };
        write!(f, "opus: {}", message.to_string_lossy())
    }
}

impl std::error::Error for OpusError {}

/// Turns a libopus return value into a count or an error.
fn check(code: c_int) -> Result<usize, OpusError> {
    usize::try_from(code).map_err(|_| OpusError(code))
}

/// Encodes one frame at a time.
pub struct Encoder {
    raw: NonNull<ffi::OpusEncoder>,
    frame: FrameSize,
}

// SAFETY: an OpusEncoder is plain memory owned by this value, with no thread
// affinity; &mut self on every call keeps access exclusive.
unsafe impl Send for Encoder {}

impl Encoder {
    /// A mono 48 kHz encoder for `frame`-sized frames at `bitrate` bits per
    /// second.
    pub fn new(frame: FrameSize, bitrate: u32) -> Result<Encoder, OpusError> {
        let mut error = 0;
        // SAFETY: the arguments are valid constants, and `error` is a valid
        // place for libopus to write its result code.
        let raw = unsafe {
            ffi::opus_encoder_create(
                SAMPLE_RATE as i32,
                1,
                ffi::OPUS_APPLICATION_RESTRICTED_LOWDELAY,
                &mut error,
            )
        };
        let raw = NonNull::new(raw).ok_or(OpusError(error))?;
        let encoder = Encoder { raw, frame };
        let bitrate = c_int::try_from(bitrate).map_err(|_| OpusError(ffi::OPUS_BAD_ARG))?;
        // SAFETY: the encoder is valid, and OPUS_SET_BITRATE takes one int.
        check(unsafe {
            ffi::opus_encoder_ctl(encoder.raw.as_ptr(), ffi::OPUS_SET_BITRATE_REQUEST, bitrate)
        })?;
        Ok(encoder)
    }

    /// The frame size this encoder was made for.
    pub fn frame_size(&self) -> FrameSize {
        self.frame
    }

    /// How many samples the encoder delays the signal by, on top of the
    /// frame itself. Part of the mouth-to-ear latency.
    pub fn lookahead(&self) -> Result<usize, OpusError> {
        let mut samples: i32 = 0;
        // SAFETY: the encoder is valid, and OPUS_GET_LOOKAHEAD writes one int
        // through the pointer it is given.
        check(unsafe {
            ffi::opus_encoder_ctl(
                self.raw.as_ptr(),
                ffi::OPUS_GET_LOOKAHEAD_REQUEST,
                &mut samples as *mut i32,
            )
        })?;
        check(samples)
    }

    /// Encodes one frame of samples in `[-1, 1]` and returns the Opus
    /// packet. `pcm` must hold exactly one frame.
    pub fn encode(&mut self, pcm: &[f32]) -> Result<Vec<u8>, OpusError> {
        let mut out = Vec::with_capacity(MAX_FRAME_BYTES);
        self.encode_into(pcm, &mut out)?;
        Ok(out)
    }

    /// Like [`Encoder::encode`], but writes the Opus packet into `out`,
    /// replacing what it held. Nothing is allocated when `out` has a
    /// capacity of at least [`MAX_FRAME_BYTES`].
    pub fn encode_into(&mut self, pcm: &[f32], out: &mut Vec<u8>) -> Result<(), OpusError> {
        assert_eq!(
            pcm.len(),
            self.frame.samples(),
            "encode takes exactly one frame"
        );
        out.clear();
        out.resize(MAX_FRAME_BYTES, 0);
        // SAFETY: `pcm` holds one frame of mono samples, as the frame size
        // argument says, and `out` is writable for the length passed.
        let result = check(unsafe {
            ffi::opus_encode_float(
                self.raw.as_ptr(),
                pcm.as_ptr(),
                pcm.len() as c_int,
                out.as_mut_ptr(),
                out.len() as i32,
            )
        });
        out.truncate(*result.as_ref().unwrap_or(&0));
        result.map(|_| ())
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: the pointer came from opus_encoder_create and is freed once.
        unsafe { ffi::opus_encoder_destroy(self.raw.as_ptr()) }
    }
}

/// Decodes one sender's frames, and conceals the ones that never arrived.
///
/// Opus decoding is stateful: keep one decoder per sender and feed it every
/// frame in order, calling [`Decoder::conceal`] for each missing one.
pub struct Decoder {
    raw: NonNull<ffi::OpusDecoder>,
}

// SAFETY: as for Encoder.
unsafe impl Send for Decoder {}

impl Decoder {
    /// A mono 48 kHz decoder.
    pub fn new() -> Result<Decoder, OpusError> {
        let mut error = 0;
        // SAFETY: valid constants, and a valid place for the result code.
        let raw = unsafe { ffi::opus_decoder_create(SAMPLE_RATE as i32, 1, &mut error) };
        let raw = NonNull::new(raw).ok_or(OpusError(error))?;
        Ok(Decoder { raw })
    }

    /// Decodes one Opus packet into `pcm` and returns how many samples it
    /// wrote. `pcm` must be at least as long as the packet's frame.
    pub fn decode(&mut self, packet: &[u8], pcm: &mut [f32]) -> Result<usize, OpusError> {
        // SAFETY: `packet` is readable for its length, and `pcm` writable for
        // the frame size passed; libopus checks the packet fits in it.
        check(unsafe {
            ffi::opus_decode_float(
                self.raw.as_ptr(),
                packet.as_ptr(),
                packet.len() as i32,
                pcm.as_mut_ptr(),
                pcm.len() as c_int,
                0,
            )
        })
    }

    /// Fills `pcm` with a plausible continuation of the signal, for a frame
    /// that never arrived (packet loss concealment). `pcm` must hold exactly
    /// one frame of a supported size.
    pub fn conceal(&mut self, pcm: &mut [f32]) -> Result<usize, OpusError> {
        assert!(
            FrameSize::from_samples(pcm.len()).is_some(),
            "conceal takes exactly one frame"
        );
        // SAFETY: a null packet asks libopus for concealment; `pcm` is
        // writable for the frame size passed.
        check(unsafe {
            ffi::opus_decode_float(
                self.raw.as_ptr(),
                std::ptr::null(),
                0,
                pcm.as_mut_ptr(),
                pcm.len() as c_int,
                0,
            )
        })
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: the pointer came from opus_decoder_create and is freed once.
        unsafe { ffi::opus_decoder_destroy(self.raw.as_ptr()) }
    }
}

/// How many samples an Opus packet holds, read from its first byte. A
/// receiver uses it to know a sender's frame size before decoding.
pub fn packet_samples(packet: &[u8]) -> Result<usize, OpusError> {
    // SAFETY: `packet` is readable for the length passed.
    check(unsafe {
        ffi::opus_packet_get_nb_samples(packet.as_ptr(), packet.len() as i32, SAMPLE_RATE as i32)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frequency: f32, amplitude: f32, samples: usize) -> Vec<f32> {
        (0..samples)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                amplitude * (2.0 * std::f32::consts::PI * frequency * t).sin()
            })
            .collect()
    }

    /// Encodes then decodes `input` frame by frame.
    fn round_trip(frame: FrameSize, input: &[f32]) -> Vec<f32> {
        let mut encoder = Encoder::new(frame, 64_000).unwrap();
        let mut decoder = Decoder::new().unwrap();
        let mut output = Vec::with_capacity(input.len());
        let mut pcm = vec![0.0; frame.samples()];
        for chunk in input.chunks_exact(frame.samples()) {
            let packet = encoder.encode(chunk).unwrap();
            assert!(packet.len() <= MAX_FRAME_BYTES);
            assert_eq!(packet_samples(&packet), Ok(frame.samples()));
            // RFC 6716 section 3.1: TOC configurations 16 to 31 are CELT-only.
            assert!(packet[0] >> 3 >= 16, "{frame}: not a CELT-only frame");
            assert_eq!(decoder.decode(&packet, &mut pcm), Ok(frame.samples()));
            output.extend_from_slice(&pcm);
        }
        output
    }

    /// The delay, in samples, at which `output` best matches `input`.
    fn best_lag(input: &[f32], output: &[f32], max_lag: usize) -> usize {
        let correlation =
            |lag: usize| -> f32 { input.iter().zip(&output[lag..]).map(|(a, b)| a * b).sum() };
        (0..max_lag)
            .max_by(|a, b| correlation(*a).total_cmp(&correlation(*b)))
            .unwrap()
    }

    #[test]
    fn every_frame_size_round_trips_with_2_5_ms_of_lookahead() {
        // Two tones mixed, so that a wrong lag cannot line up by accident.
        let n = SAMPLE_RATE as usize / 2;
        let input: Vec<f32> = sine(440.0, 0.3, n)
            .iter()
            .zip(sine(1234.0, 0.2, n))
            .map(|(a, b)| a + b)
            .collect();

        for frame in FrameSize::ALL {
            let encoder = Encoder::new(frame, 64_000).unwrap();
            let lookahead = encoder.lookahead().unwrap();
            // Measured, and pinned: RESTRICTED_LOWDELAY adds 2.5 ms.
            assert_eq!(lookahead, 120, "{frame}");

            let output = round_trip(frame, &input);
            assert_eq!(best_lag(&input, &output, 480), lookahead, "{frame}");

            // After that delay the decoded signal is close to the input:
            // at least 15 dB of signal to noise ratio.
            let skip = 2 * 480; // let the codec settle
            let (signal, noise) = input[skip..n - lookahead]
                .iter()
                .zip(&output[skip + lookahead..])
                .fold((0.0f32, 0.0f32), |(s, e), (a, b)| {
                    (s + a * a, e + (a - b) * (a - b))
                });
            let snr_db = 10.0 * (signal / noise).log10();
            assert!(snr_db > 15.0, "{frame}: SNR {snr_db:.1} dB");
        }
    }

    #[test]
    fn concealment_fills_one_frame_and_fades() {
        let frame = FrameSize::Ms5;
        let mut encoder = Encoder::new(frame, 64_000).unwrap();
        let mut decoder = Decoder::new().unwrap();
        let mut pcm = vec![0.0; frame.samples()];
        for chunk in sine(300.0, 0.5, 20 * frame.samples()).chunks_exact(frame.samples()) {
            decoder
                .decode(&encoder.encode(chunk).unwrap(), &mut pcm)
                .unwrap();
        }
        let energy = |pcm: &[f32]| pcm.iter().map(|x| x * x).sum::<f32>();

        assert_eq!(decoder.conceal(&mut pcm), Ok(frame.samples()));
        let first = energy(&pcm);
        assert!(first > 0.0, "concealment continues the signal");
        for _ in 0..40 {
            decoder.conceal(&mut pcm).unwrap();
        }
        assert!(energy(&pcm) < first / 10.0, "long concealment fades out");
    }

    #[test]
    fn errors_come_from_libopus() {
        assert!(packet_samples(&[]).is_err());
        let mut decoder = Decoder::new().unwrap();
        let mut pcm = vec![0.0; 120];
        let err = decoder.decode(&[0xff, 0xff, 0xff], &mut pcm).unwrap_err();
        assert!(err.to_string().starts_with("opus: "), "{err}");
    }
}
