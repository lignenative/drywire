//! Just enough of the WAV format to read a test recording and write what a
//! client played: 48 kHz only, any channel count (mixed down to mono on
//! reading), 16-, 24- or 32-bit integer or 32-bit float samples. Files are
//! written as mono 32-bit float, which every audio tool opens.
//!
//! A WAV file is a RIFF container: `"RIFF" size "WAVE"`, then chunks of
//! `id (4) ‖ size (4, little-endian) ‖ data`, padded to an even size. The
//! `fmt ` chunk describes the samples, the `data` chunk holds them.

use std::fmt;
use std::io::Write;
use std::path::Path;

use crate::opus::SAMPLE_RATE;

const FORMAT_PCM: u16 = 1;
const FORMAT_FLOAT: u16 = 3;
const FORMAT_EXTENSIBLE: u16 = 0xfffe;

/// Why a WAV file could not be read.
#[derive(Debug)]
pub struct WavError(String);

impl fmt::Display for WavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WavError {}

fn error(message: impl Into<String>) -> WavError {
    WavError(message.into())
}

/// Reads a 48 kHz WAV file as mono samples in `[-1, 1]`.
pub fn read(path: &Path) -> Result<Vec<f32>, WavError> {
    let bytes = std::fs::read(path).map_err(|e| error(format!("{}: {e}", path.display())))?;
    decode(&bytes).map_err(|e| error(format!("{}: {e}", path.display())))
}

/// Decodes the bytes of a WAV file as mono samples.
pub fn decode(bytes: &[u8]) -> Result<Vec<f32>, WavError> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(error("not a WAV file"));
    }
    let mut format = None;
    let mut data = None;
    let mut rest = &bytes[12..];
    while rest.len() >= 8 {
        let id = &rest[..4];
        let size = u32::from_le_bytes(rest[4..8].try_into().expect("4 bytes")) as usize;
        let body = rest
            .get(8..8 + size)
            .ok_or_else(|| error("a chunk runs past the end of the file"))?;
        match id {
            b"fmt " => format = Some(Format::parse(body)?),
            b"data" => data = Some(body),
            _ => {}
        }
        rest = rest.get(8 + size + size % 2..).unwrap_or(&[]);
    }
    let format = format.ok_or_else(|| error("no fmt chunk"))?;
    let data = data.ok_or_else(|| error("no data chunk"))?;
    if format.rate != SAMPLE_RATE {
        return Err(error(format!(
            "the sample rate is {} Hz; Drywire works at {SAMPLE_RATE} Hz, so convert it first",
            format.rate
        )));
    }
    let width = usize::from(format.bits / 8);
    let frame = width * usize::from(format.channels);
    Ok(data
        .chunks_exact(frame)
        .map(|f| {
            let sum: f32 = f.chunks_exact(width).map(|s| format.sample(s)).sum();
            sum / f32::from(format.channels)
        })
        .collect())
}

struct Format {
    float: bool,
    channels: u16,
    rate: u32,
    bits: u16,
}

impl Format {
    fn parse(body: &[u8]) -> Result<Format, WavError> {
        if body.len() < 16 {
            return Err(error("fmt chunk too short"));
        }
        let u16_at = |i: usize| u16::from_le_bytes([body[i], body[i + 1]]);
        let mut tag = u16_at(0);
        if tag == FORMAT_EXTENSIBLE {
            // The real format is the first two bytes of the sub-format GUID.
            if body.len() < 26 {
                return Err(error("extensible fmt chunk too short"));
            }
            tag = u16_at(24);
        }
        let format = Format {
            float: tag == FORMAT_FLOAT,
            channels: u16_at(2),
            rate: u32::from_le_bytes(body[4..8].try_into().expect("4 bytes")),
            bits: u16_at(14),
        };
        let supported = match tag {
            FORMAT_PCM => matches!(format.bits, 16 | 24 | 32),
            FORMAT_FLOAT => format.bits == 32,
            _ => false,
        };
        if !supported || format.channels == 0 {
            return Err(error(format!(
                "unsupported sample format (tag {tag}, {} bits, {} channels)",
                format.bits, format.channels
            )));
        }
        Ok(format)
    }

    /// One sample, little-endian, scaled to `[-1, 1]`.
    fn sample(&self, b: &[u8]) -> f32 {
        match (self.float, self.bits) {
            (true, _) => f32::from_le_bytes(b.try_into().expect("4 bytes")),
            (false, 16) => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0,
            // Put the 24 bits at the top of an i32, so the sign is right.
            (false, 24) => i32::from_le_bytes([0, b[0], b[1], b[2]]) as f32 / 2_147_483_648.0,
            (false, _) => {
                i32::from_le_bytes(b.try_into().expect("4 bytes")) as f32 / 2_147_483_648.0
            }
        }
    }
}

/// Encodes mono 48 kHz samples as a 32-bit float WAV file.
pub fn encode(samples: &[f32]) -> Vec<u8> {
    let data_size = (samples.len() * 4) as u32;
    let mut b = Vec::with_capacity(44 + samples.len() * 4);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_size).to_le_bytes());
    b.extend_from_slice(b"WAVE");
    b.extend_from_slice(b"fmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&FORMAT_FLOAT.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // channels
    b.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    b.extend_from_slice(&(SAMPLE_RATE * 4).to_le_bytes()); // bytes per second
    b.extend_from_slice(&4u16.to_le_bytes()); // bytes per frame
    b.extend_from_slice(&32u16.to_le_bytes()); // bits per sample
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_size.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

/// Writes mono 48 kHz samples to a WAV file.
pub fn write(path: &Path, samples: &[f32]) -> std::io::Result<()> {
    std::fs::File::create(path)?.write_all(&encode(samples))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_files_round_trip() {
        let samples = [0.0, 0.5, -0.25, 1.0];
        assert_eq!(decode(&encode(&samples)).unwrap(), samples);
    }

    /// A 16-bit stereo file, written by hand: mixed down to mono.
    #[test]
    fn integer_stereo_is_mixed_down() {
        let mut b = encode(&[]);
        // Patch the fmt chunk: PCM, 2 channels, 16 bits.
        b[20..22].copy_from_slice(&FORMAT_PCM.to_le_bytes());
        b[22..24].copy_from_slice(&2u16.to_le_bytes());
        b[34..36].copy_from_slice(&16u16.to_le_bytes());
        let frames: [i16; 4] = [16384, 0, -32768, -32768];
        b.truncate(40);
        b.extend_from_slice(&8u32.to_le_bytes());
        for s in frames {
            b.extend_from_slice(&s.to_le_bytes());
        }
        assert_eq!(decode(&b).unwrap(), [0.25, -1.0]);
    }

    #[test]
    fn other_rates_are_refused() {
        let mut b = encode(&[0.0]);
        b[24..28].copy_from_slice(&44_100u32.to_le_bytes());
        let err = decode(&b).unwrap_err();
        assert!(err.to_string().contains("44100 Hz"), "{err}");
        assert!(decode(b"RIFF").is_err());
    }
}
