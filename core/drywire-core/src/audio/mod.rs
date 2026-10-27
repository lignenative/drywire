//! Audio input and output behind one small trait, with one backend per
//! operating system (spec section 6).
//!
//! Every backend speaks the same format to the rest of the core: mono,
//! 32-bit float samples in `[-1, 1]`, at 48 kHz. Converting to what the
//! device wants (channels, sample rate) is the backend's job. Each backend
//! asks the OS for its lowest-latency mode and never enables any voice
//! processing: no echo cancellation, no gain control, no noise suppression.
//!
//! | Backend | Where | What it opens |
//! | --- | --- | --- |
//! | [`file::FileIo`] | everywhere | No device: samples from memory, paced in real time, for tests and bots |
//!
//! The callbacks run on the backend's real-time threads. They must return
//! quickly and should neither block nor wait on a lock another thread holds
//! for long.

use std::fmt;

pub mod file;

pub use crate::opus::SAMPLE_RATE;

/// Receives captured samples: mono, 48 kHz.
pub type InputCallback = Box<dyn FnMut(&[f32]) + Send + 'static>;
/// Fills the samples to play: mono, 48 kHz.
pub type OutputCallback = Box<dyn FnMut(&mut [f32]) + Send + 'static>;

/// Capture or playback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// From a microphone.
    Input,
    /// To a speaker or earphone.
    Output,
}

/// What the operating system actually gave for one direction. These are the
/// numbers that end up in the latency budget (spec section 4), as reported,
/// not measured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamInfo {
    /// Capture or playback.
    pub direction: Direction,
    /// The device's name.
    pub device: String,
    /// Which API and mode serve it, for example "WASAPI exclusive".
    pub mode: String,
    /// The device's own sample rate. When it is not 48 kHz the OS
    /// resamples, which adds a little latency.
    pub device_rate: u32,
    /// The I/O buffer, in frames at the device rate.
    pub buffer_frames: u32,
    /// The smallest I/O buffer the device accepts.
    pub min_buffer_frames: u32,
    /// The latency the OS reports on top of the buffer (device, stream and
    /// safety offset on macOS; stream latency on Windows), in frames at the
    /// device rate.
    pub latency_frames: u32,
    /// What `latency_frames` is made of, as the OS names the parts.
    pub latency_parts: Vec<(&'static str, u32)>,
    /// Whether the device is a Bluetooth one, which the spec warns about.
    pub bluetooth: bool,
}

impl StreamInfo {
    /// The buffer in milliseconds.
    pub fn buffer_ms(&self) -> f64 {
        frames_to_ms(self.buffer_frames, self.device_rate)
    }

    /// The reported latency on top of the buffer, in milliseconds.
    pub fn latency_ms(&self) -> f64 {
        frames_to_ms(self.latency_frames, self.device_rate)
    }
}

fn frames_to_ms(frames: u32, rate: u32) -> f64 {
    if rate == 0 {
        return 0.0;
    }
    f64::from(frames) * 1000.0 / f64::from(rate)
}

impl fmt::Display for StreamInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let direction = match self.direction {
            Direction::Input => "input ",
            Direction::Output => "output",
        };
        write!(
            f,
            "{direction} {:?} via {}: {} Hz, buffer {} frames ({:.2} ms, smallest {}), \
             reported latency {} frames ({:.2} ms",
            self.device,
            self.mode,
            self.device_rate,
            self.buffer_frames,
            self.buffer_ms(),
            self.min_buffer_frames,
            self.latency_frames,
            self.latency_ms(),
        )?;
        for (i, (name, frames)) in self.latency_parts.iter().enumerate() {
            let sep = if i == 0 { ": " } else { " + " };
            write!(f, "{sep}{name} {frames}")?;
        }
        f.write_str(")")?;
        if self.bluetooth {
            f.write_str(" — Bluetooth: expect a lot more latency than reported")?;
        }
        Ok(())
    }
}

/// Why audio could not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioError(pub String);

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AudioError {}

/// What to ask of a device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AudioOptions {
    /// The I/O buffer to ask for, in frames. `None` asks for the smallest
    /// the device accepts.
    pub buffer_frames: Option<u32>,
}

/// A pair of audio streams, capture and playback.
///
/// [`AudioIo::start`] opens both and calls the callbacks until
/// [`AudioIo::stop`], or until the value is dropped.
pub trait AudioIo {
    /// Starts capture and playback, and returns what the OS gave for each.
    fn start(
        &mut self,
        input: InputCallback,
        output: OutputCallback,
    ) -> Result<Vec<StreamInfo>, AudioError>;

    /// Stops both streams. Calling it twice, or before `start`, does nothing.
    fn stop(&mut self);
}

/// The backend for this operating system's default devices.
pub fn default_devices(options: AudioOptions) -> Result<Box<dyn AudioIo>, AudioError> {
    let _ = options;
    Err(AudioError(
        "no audio backend for this operating system yet; use files".into(),
    ))
}
