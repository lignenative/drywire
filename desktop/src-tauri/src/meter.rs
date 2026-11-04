//! The microphone's level out of a call, for the meter in the settings:
//! the person sets the voice gate's threshold against their own voice
//! before joining anyone.
//!
//! The interface polls [`Meter::level`] while the meter is on screen. The
//! first poll opens the sound card on a thread of its own (a device may
//! not be movable between threads), and the thread closes it by itself a
//! second after the last poll: a hidden window or another page stops
//! polling, and the microphone is not held open for nothing. A call or
//! the latency lab stops the meter first, since they need the card.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use drywire_core::lab::LevelMeter;

use crate::session::{DeviceFactory, METER_FALL_DB, METER_FLOOR_DBFS, TICK};

/// How long the sound card stays open after the last poll.
pub const IDLE: Duration = Duration::from_secs(1);

/// What the meter's thread and the polls share.
struct Shared {
    /// The level shown, in dBFS, as `f32` bits: it rises at once and falls
    /// slowly, as in a call.
    level: AtomicU32,
    /// When the interface last asked.
    asked: Mutex<Instant>,
    /// Set to close the sound card now.
    stop: AtomicBool,
    /// Why the sound card could not be opened, if it could not.
    failed: Mutex<Option<String>>,
}

/// A running meter. Dropping it closes the sound card and waits for its
/// thread.
pub struct Meter {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Meter {
    /// Opens the sound card that `device` makes, on a thread of its own,
    /// and starts measuring. Returns at once: the level stays at the floor
    /// until the device runs.
    pub fn start(device: DeviceFactory) -> Result<Meter, String> {
        let shared = Arc::new(Shared {
            level: AtomicU32::new(METER_FLOOR_DBFS.to_bits()),
            asked: Mutex::new(Instant::now()),
            stop: AtomicBool::new(false),
            failed: Mutex::new(None),
        });
        let thread = {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("drywire-meter".into())
                .spawn(move || run(&shared, device))
                .map_err(|e| e.to_string())?
        };
        Ok(Meter {
            shared,
            thread: Some(thread),
        })
    }

    /// The level now, in dBFS from [`METER_FLOOR_DBFS`] up; keeps the
    /// sound card open for another [`IDLE`]. `Err` once the sound card
    /// failed to open.
    pub fn level(&self) -> Result<f32, String> {
        if let Some(why) = self.shared.failed.lock().expect("not poisoned").clone() {
            return Err(why);
        }
        *self.shared.asked.lock().expect("not poisoned") = Instant::now();
        Ok(f32::from_bits(self.shared.level.load(Ordering::Relaxed)))
    }

    /// Whether the thread stopped on its own: nobody asked for a while,
    /// or the sound card failed. A new meter is then needed.
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The meter's thread: opens the device, reads the peak every tick, and
/// closes the device once stopped or no longer asked.
fn run(shared: &Shared, device: DeviceFactory) {
    let fail = |why: String| *shared.failed.lock().expect("not poisoned") = Some(why);
    let (meter, input, output) = LevelMeter::new();
    let mut device = match device() {
        Ok(device) => device,
        Err(e) => return fail(e),
    };
    if let Err(e) = device.start(input, output) {
        return fail(e.to_string());
    }
    let mut level = METER_FLOOR_DBFS;
    loop {
        std::thread::sleep(TICK);
        let idle = shared.asked.lock().expect("not poisoned").elapsed() > IDLE;
        if idle || shared.stop.load(Ordering::Relaxed) {
            break;
        }
        let peak = meter.take_peak_dbfs().max(METER_FLOOR_DBFS);
        level = peak.max(level - METER_FALL_DB);
        shared.level.store(level.to_bits(), Ordering::Relaxed);
    }
    device.stop();
}
