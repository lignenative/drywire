//! A device with no hardware: capture comes from samples in memory, and
//! playback goes to memory, both paced in real time by one thread.
//!
//! It exists so that a whole call can run headless, in tests and on
//! machines with no sound card, without ever asking for the microphone. It
//! is also the echo bot of `make live`: in [`Source::Echo`] mode, what it
//! plays is what it captures next, so a person on the other end hears
//! themselves come back through the relay.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{
    AudioError, AudioIo, Direction, InputCallback, OutputCallback, SAMPLE_RATE, StreamInfo,
};

/// What the device captures.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// These samples, then silence.
    Samples(Vec<f32>),
    /// What it played one period earlier.
    Echo,
}

/// A paced, in-memory audio device.
pub struct FileIo {
    source: Source,
    period: usize,
    duration: Option<Duration>,
    recorded: Arc<Mutex<Vec<f32>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FileIo {
    /// A device that captures from `source` and calls both callbacks every
    /// `period` samples. It runs for `duration`, or until stopped.
    pub fn new(source: Source, period: usize, duration: Option<Duration>) -> FileIo {
        assert!(period > 0, "a period holds at least one sample");
        FileIo {
            source,
            period,
            duration,
            recorded: Arc::new(Mutex::new(Vec::new())),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }

    /// Everything played so far.
    pub fn recorded(&self) -> Vec<f32> {
        self.recorded.lock().expect("not poisoned").clone()
    }

    /// Whether the duration has run out (always false without one).
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_some_and(JoinHandle::is_finished)
    }
}

impl AudioIo for FileIo {
    fn start(
        &mut self,
        mut input: InputCallback,
        mut output: OutputCallback,
    ) -> Result<Vec<StreamInfo>, AudioError> {
        if self.thread.is_some() {
            return Err(AudioError("already started".into()));
        }
        let source = self.source.clone();
        let period = self.period;
        let total = self
            .duration
            .map(|d| (d.as_secs_f64() * f64::from(SAMPLE_RATE)) as usize);
        let recorded = Arc::clone(&self.recorded);
        let stop = Arc::clone(&self.stop);
        let mut captured = vec![0.0f32; period];
        let mut played = vec![0.0f32; period];

        let thread = std::thread::Builder::new()
            .name("drywire-file-audio".into())
            .spawn(move || {
                let start = Instant::now();
                let mut position = 0usize;
                while !stop.load(Ordering::Relaxed) && total.is_none_or(|t| position < t) {
                    // The period is captured once it has fully elapsed, as
                    // with a real microphone.
                    let due = start + period_duration(position + period);
                    std::thread::sleep(due.saturating_duration_since(Instant::now()));

                    match &source {
                        Source::Samples(samples) => {
                            captured.fill(0.0);
                            let end = (position + period).min(samples.len());
                            if position < end {
                                captured[..end - position].copy_from_slice(&samples[position..end]);
                            }
                        }
                        Source::Echo => captured.copy_from_slice(&played),
                    }
                    input(&captured);
                    output(&mut played);
                    recorded
                        .lock()
                        .expect("not poisoned")
                        .extend_from_slice(&played);
                    position += period;
                }
            })
            .map_err(|e| AudioError(format!("start the file audio thread: {e}")))?;
        self.thread = Some(thread);

        let info = |direction| StreamInfo {
            direction,
            device: "file".into(),
            mode: "in-memory, paced in real time".into(),
            device_rate: SAMPLE_RATE,
            buffer_frames: period as u32,
            min_buffer_frames: 1,
            latency_frames: 0,
            latency_parts: Vec::new(),
            bluetooth: false,
        };
        Ok(vec![info(Direction::Input), info(Direction::Output)])
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for FileIo {
    fn drop(&mut self) {
        self.stop();
    }
}

fn period_duration(samples: usize) -> Duration {
    Duration::from_micros(samples as u64 * 1_000_000 / u64::from(SAMPLE_RATE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plays_back_what_the_callbacks_produce_in_real_time() {
        let source = Source::Samples(vec![0.5; 480]);
        let mut io = FileIo::new(source, 240, Some(Duration::from_millis(20)));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_by_input = Arc::clone(&seen);
        let started = Instant::now();
        io.start(
            Box::new(move |s: &[f32]| seen_by_input.lock().unwrap().extend_from_slice(s)),
            Box::new(|out: &mut [f32]| out.fill(0.25)),
        )
        .unwrap();
        while !io.is_finished() {
            std::thread::sleep(Duration::from_millis(1));
        }
        io.stop();
        assert!(started.elapsed() >= Duration::from_millis(20), "paced");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 960);
        assert_eq!(&seen[..480], &[0.5; 480], "the samples");
        assert_eq!(&seen[480..], &[0.0; 480], "then silence");
        assert_eq!(io.recorded(), vec![0.25; 960]);
    }

    #[test]
    fn echo_captures_what_it_played() {
        let mut io = FileIo::new(Source::Echo, 120, Some(Duration::from_millis(10)));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_by_input = Arc::clone(&seen);
        let mut next = 0.0;
        io.start(
            Box::new(move |s: &[f32]| seen_by_input.lock().unwrap().push(s[0])),
            Box::new(move |out: &mut [f32]| {
                next += 1.0;
                out.fill(next);
            }),
        )
        .unwrap();
        while !io.is_finished() {
            std::thread::sleep(Duration::from_millis(1));
        }
        io.stop();
        assert_eq!(*seen.lock().unwrap(), [0.0, 1.0, 2.0, 3.0]);
    }
}
