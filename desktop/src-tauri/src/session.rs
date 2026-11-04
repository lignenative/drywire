//! A call in progress, run by a thread of its own: the "session thread".
//!
//! It does what `drywire join` does: follows the space's events, asks the
//! control plane for a session, starts the [`Call`] and the sound card,
//! then adds and removes members as they come and go. Every 20 ms it also
//! copies the call's counters into a [`CallView`], which the interface
//! polls. Nothing here runs on the audio path: the audio callbacks belong
//! to `drywire-core`, and this thread only reads the snapshot of
//! statistics they publish when their lock is free.
//!
//! When the server forgets this device's session, because the network
//! dropped long enough for the relay to time it out or the server
//! restarted, the session thread joins the room again on its own, with
//! growing pauses between attempts, and the interface shows it as
//! reconnecting. The person may start the waiting attempt at once
//! (ADR 0050). Mute, the gate and the volumes carry over. Only a session
//! the server ended on purpose (this device joined elsewhere, or was
//! removed) ends the call.
//!
//! The session thread owns the sound card, because a device may not be
//! movable between threads; it stops it before the call, as the core
//! requires. The interface's controls reach the call without it: mute
//! and the gate through the core's [`CallControls`] atomics, volumes
//! through a channel the thread reads at each tick, since they follow a
//! person (a public key) and only this thread knows their sessions.

use std::collections::{BTreeMap, HashMap};
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use drywire_control::{Client, Event, Id, Presence, to_hex};
use drywire_core::audio::{AudioIo, StreamInfo};
use drywire_core::call::{Call, CallControls, CallMembers, CallOptions, CallStats};
use drywire_core::config::{ClientConfig, Member};
use drywire_core::encryption::{EncryptionState, SenderState};
use drywire_core::gate::GateConfig;
use drywire_core::link::LinkConfig;
use drywire_core::opus::{FrameSize, SAMPLE_RATE};
use drywire_core::sender::SenderConfig;
use drywire_proto::crypto::{SigningKey, VerifyingKey};
use serde::Serialize;

use crate::error::AppError;
use crate::latency::estimate_ms;
use crate::store::Settings;

/// Opens the sound card, on the session thread.
pub type DeviceFactory = Box<dyn FnOnce() -> Result<Box<dyn AudioIo>, String> + Send>;

/// Makes the factory that opens the sound card: once per call, and again
/// for every reconnection.
pub type Devices = Arc<dyn Fn() -> DeviceFactory + Send + Sync>;

/// Hears who comes and goes in the room, on the session thread: the app
/// plays a sound and shows a pill for each (spec section 10, "Presence").
pub type Notices = Arc<dyn Fn(Notice) + Send + Sync>;

/// Someone came into the room, or went.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    /// `join`, `leave`, or `disconnect`: the relay stopped hearing from
    /// them.
    pub kind: &'static str,
    /// Their display name.
    pub name: String,
}

/// How often the session thread wakes up: to apply the space's events and
/// refresh the [`CallView`].
pub(crate) const TICK: Duration = Duration::from_millis(20);

/// How long to wait before opening the event stream again after it ends.
const EVENTS_RETRY: Duration = Duration::from_secs(2);

/// The pause before each attempt to join again, then the longest pause:
/// quick at first, for a blip, then every 15 seconds. An attempt the
/// person starts restarts the sequence (ADR 0050).
const RECONNECT_FIRST: Duration = Duration::from_secs(1);
const RECONNECT_MAX: Duration = Duration::from_secs(15);

/// How far below the open threshold the gate closes: the core's default
/// hysteresis, so one setting moves both.
pub const GATE_HYSTERESIS_DB: f32 = 6.0;

/// The bottom of the level meter, in dBFS: quieter shows as empty.
pub const METER_FLOOR_DBFS: f32 = -90.0;

/// How fast the meter falls, per tick: 50 dB a second, so a word's peak
/// stays visible between two polls of the interface.
pub(crate) const METER_FALL_DB: f32 = 1.0;

/// How many ticks the loss figures are measured over: one second.
const QUALITY_TICKS: u32 = 50;

/// A threshold the interface may set, in dBFS.
pub fn clamp_gate(open_dbfs: f32) -> f32 {
    if open_dbfs.is_nan() {
        return GateConfig::default().open_dbfs;
    }
    open_dbfs.clamp(-80.0, -10.0)
}

/// The gate for an open threshold.
fn gate_config(open_dbfs: f32) -> GateConfig {
    let open_dbfs = clamp_gate(open_dbfs);
    GateConfig {
        open_dbfs,
        close_dbfs: open_dbfs - GATE_HYSTERESIS_DB,
        ..GateConfig::default()
    }
}

/// Where to go, and as whom.
#[derive(Clone)]
pub struct JoinRequest {
    /// A client for the space's instance.
    pub client: Arc<Client>,
    /// The device's identity.
    pub identity: SigningKey,
    /// The space.
    pub space_id: Id,
    /// Its name, for display.
    pub space_name: String,
    /// The secret its members share.
    pub space_key: [u8; 32],
    /// The room to join.
    pub room_id: Id,
    /// The gate threshold and the volumes to start with.
    pub settings: Settings,
    /// Told of every arrival and departure.
    pub notices: Notices,
}

/// What the interface asks of the session thread.
enum Command {
    /// Play everyone with this public key (hex) at this volume.
    Volume(String, f32),
    /// While reconnecting, start the waiting attempt now.
    RetryNow,
}

/// Someone in the room, for display.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    /// Their session on the relay.
    pub session: u32,
    /// Their display name.
    pub name: String,
    /// Their identity's public key, in hex: what identifies a person
    /// across sessions.
    pub public_key: String,
    /// Whether this is the device itself.
    pub me: bool,
    /// Whether their voice is playing now (this device: whether its voice
    /// gate is open).
    pub speaking: bool,
    /// The volume they are played at: 1 is 100 %, 2 the most.
    pub volume: f32,
    /// The delay their jitter buffer aims for, in milliseconds, once they
    /// spoke.
    pub buffer_ms: Option<f64>,
    /// The share of their voice lost and concealed over the last second,
    /// in percent; `None` when they were silent.
    pub loss_pct: Option<f64>,
    /// Their estimated mouth-to-ear latency to this device, in
    /// milliseconds, once they spoke ([`crate::latency`]).
    pub estimate_ms: Option<f64>,
    /// Their playback counters, over the call.
    pub counters: Counters,
}

/// One person's playback counters, for the latency lab's statistics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counters {
    /// Packets received.
    pub received: u64,
    /// Frames played.
    pub played: u64,
    /// Gaps filled by concealment.
    pub concealed: u64,
    /// Packets that came after their time.
    pub late: u64,
    /// Frames dropped because too many piled up.
    pub dropped: u64,
    /// Frames in the jitter buffer now.
    pub depth: usize,
}

/// The encryption lock's state (ADR 0041).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LockState {
    /// Nothing verified yet: alone, nobody spoke so far, or someone's
    /// audio arrives before their signed announcement.
    #[default]
    Pending,
    /// Everyone heard is verified: the lock is closed.
    Verified,
    /// A forged announcement or a tampered frame in the last 10 seconds,
    /// dropped and never played.
    Failed,
}

impl From<EncryptionState> for LockState {
    fn from(state: EncryptionState) -> LockState {
        match state {
            EncryptionState::Pending => LockState::Pending,
            EncryptionState::Verified => LockState::Verified,
            EncryptionState::Failed => LockState::Failed,
        }
    }
}

/// Whether end-to-end encryption is verified for everyone heard: what the
/// lock shows.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptionView {
    /// The room's state.
    pub state: LockState,
    /// Everyone else in the room, in the order of `participants`.
    pub people: Vec<PersonLock>,
}

/// One person, for the lock.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonLock {
    /// Their identity's public key, in hex, as in [`Participant`].
    pub public_key: String,
    /// Whether their signed announcement verified with that key and
    /// nothing in their name failed lately. `false` for someone silent
    /// since this device joined, who does not hold the room back.
    pub verified: bool,
    /// Whether a packet in their name failed its signature or
    /// authentication lately: the person behind a `failed` lock, whose
    /// voice was dropped.
    pub rejected: bool,
}

/// One audio stream, as the system opened it.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    /// `input` or `output`.
    pub direction: &'static str,
    /// The device's name.
    pub name: String,
    /// Which API and mode, such as "WASAPI exclusive".
    pub mode: String,
    /// The I/O buffer, in milliseconds.
    pub buffer_ms: f64,
    /// The latency the system reports on top of the buffer.
    pub latency_ms: f64,
    /// Whether it is a Bluetooth device, which adds a lot of latency.
    pub bluetooth: bool,
}

impl DeviceView {
    /// What the system says of one stream.
    pub fn new(info: &StreamInfo) -> DeviceView {
        DeviceView {
            direction: match info.direction {
                drywire_core::audio::Direction::Input => "input",
                drywire_core::audio::Direction::Output => "output",
            },
            name: printable(&info.device),
            mode: info.mode.clone(),
            buffer_ms: info.buffer_ms(),
            latency_ms: info.latency_ms(),
            bluetooth: info.bluetooth,
        }
    }
}

/// What the interface shows about the call. A copy, refreshed every
/// [`TICK`]; reading it never touches the call itself.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallView {
    /// The instance's origin.
    pub instance: String,
    /// The space's id, in hex.
    pub space_id: String,
    /// The space's name.
    pub space_name: String,
    /// The room's id, in hex.
    pub room_id: String,
    /// The room's name.
    pub room_name: String,
    /// Why the call ended, if the server ended it. The interface then
    /// offers to join again.
    pub ended: Option<String>,
    /// Whether the server forgot this device's session and the app is
    /// joining again on its own.
    pub reconnecting: bool,
    /// While reconnecting, the seconds until the next attempt, rounded up;
    /// `None` during an attempt, and out of a reconnection.
    pub retry_in_s: Option<u32>,
    /// Whether the relay acknowledged this device and still answers.
    pub bound: bool,
    /// The last round trip to the relay, in milliseconds.
    pub rtt_ms: Option<f64>,
    /// The smallest round trip seen, in milliseconds.
    pub min_rtt_ms: Option<f64>,
    /// This device's latency, input plus output, in milliseconds: the
    /// loopback test's if it was run, else what the system reports.
    pub device_ms: f64,
    /// Whether `device_ms` was measured by the loopback test.
    pub device_measured: bool,
    /// Whether the microphone is muted.
    pub muted: bool,
    /// The microphone's level for the meter, in dBFS, from
    /// [`METER_FLOOR_DBFS`] up: it rises at once and falls slowly.
    pub input_dbfs: f32,
    /// The level at which the voice gate opens, in dBFS.
    pub gate_dbfs: f32,
    /// Whether the person chose that level, or it is still the default.
    pub gate_chosen: bool,
    /// Our packets the relay expected and never got, over the call, in
    /// percent.
    pub uplink_loss_pct: f64,
    /// Everyone in the room, this device first.
    pub participants: Vec<Participant>,
    /// The audio streams, as the system opened them.
    pub devices: Vec<DeviceView>,
    /// Whether everyone heard is verified: the lock.
    pub encryption: EncryptionView,
}

/// A running session. Dropping it ends the call.
pub struct Session {
    view: Arc<Mutex<CallView>>,
    /// The running call's controls; replaced when a reconnection starts a
    /// new call.
    controls: Arc<Mutex<CallControls>>,
    /// Whether the person moved the gate during this session.
    gate_moved: Arc<AtomicBool>,
    /// Set while the session thread tries to join again: a network call
    /// that may block for many seconds.
    attempting: Arc<AtomicBool>,
    commands: Sender<Command>,
    stop: Arc<AtomicBool>,
    /// Whether ending should also tell the control plane: not when
    /// switching rooms, where the next join replaces the session anyway.
    leave_server: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    /// Joins the room and starts the sound card that `devices` opens.
    /// Returns once audio runs, or with why it could not start.
    pub fn start(request: JoinRequest, devices: Devices) -> Result<Session, AppError> {
        let view = Arc::new(Mutex::new(CallView::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let leave_server = Arc::new(AtomicBool::new(true));
        let gate_moved = Arc::new(AtomicBool::new(false));
        let attempting = Arc::new(AtomicBool::new(false));
        let (ready, started) = mpsc::channel();
        let (commands, inbox) = mpsc::channel();
        let thread = {
            let (view, stop, leave_server, gate_moved, attempting) = (
                Arc::clone(&view),
                Arc::clone(&stop),
                Arc::clone(&leave_server),
                Arc::clone(&gate_moved),
                Arc::clone(&attempting),
            );
            std::thread::Builder::new()
                .name("drywire-session".into())
                .spawn(move || {
                    let mut events = None;
                    let first = Live::start(request.clone(), devices(), &stop, false, &mut events);
                    let live = match first {
                        Ok(live) => live,
                        Err(e) => {
                            let _ = ready.send(Err(e));
                            return;
                        }
                    };
                    let controls = Arc::new(Mutex::new(live.controls.clone()));
                    let mut thread = SessionThread {
                        request,
                        devices,
                        inbox,
                        view,
                        controls: Arc::clone(&controls),
                        gate_moved,
                        stop: Arc::clone(&stop),
                        attempting,
                        events: None,
                        live: Some(live),
                        retry: None,
                    };
                    thread.publish();
                    let _ = ready.send(Ok(controls));
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(TICK);
                        thread.tick();
                    }
                    if let Some(live) = thread.live {
                        let _ = live.end(leave_server.load(Ordering::Relaxed));
                    }
                })
                .map_err(|e| AppError::new("audio", e.to_string()))?
        };
        match started.recv() {
            Ok(Ok(controls)) => Ok(Session {
                view,
                controls,
                gate_moved,
                attempting,
                commands,
                stop,
                leave_server,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(AppError::new("audio", "the call stopped while starting"))
            }
        }
    }

    /// A copy of what the interface shows.
    pub fn view(&self) -> CallView {
        let mut view = self.view.lock().expect("not poisoned").clone();
        // Mute and the gate straight from the call's controls: the copy
        // the session thread refreshes may predate a change made a moment
        // ago, and would show it undone until the next tick.
        let controls = self.controls.lock().expect("not poisoned");
        view.muted = controls.is_muted();
        view.gate_dbfs = controls.gate().0;
        view.gate_chosen |= self.gate_moved.load(Ordering::Relaxed);
        view
    }

    /// Mutes or unmutes the microphone, at the capture callback's next
    /// call.
    pub fn set_muted(&self, muted: bool) {
        self.controls.lock().expect("not poisoned").set_muted(muted);
    }

    /// Moves the voice gate's threshold (clamped to −80 through −10 dBFS).
    pub fn set_gate(&self, open_dbfs: f32) {
        let gate = gate_config(open_dbfs);
        self.controls
            .lock()
            .expect("not poisoned")
            .set_gate(gate.open_dbfs, gate.close_dbfs);
        self.gate_moved.store(true, Ordering::Relaxed);
    }

    /// Plays the person with this public key (hex) at `volume`, within a
    /// tick.
    pub fn set_volume(&self, public_key: &str, volume: f32) {
        let _ = self
            .commands
            .send(Command::Volume(public_key.to_string(), volume));
    }

    /// While reconnecting, makes the waiting attempt start at the next
    /// tick instead of after its pause. Does nothing during an attempt,
    /// which is already what was asked, nor in a running call.
    pub fn retry_now(&self) {
        let _ = self.commands.send(Command::RetryNow);
    }

    /// Ends the call. With `tell_server`, the control plane frees the
    /// room at once rather than after the relay's timeout.
    pub fn end(mut self, tell_server: bool) {
        self.leave_server.store(tell_server, Ordering::Relaxed);
        self.stop_thread();
    }

    fn stop_thread(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let Some(thread) = self.thread.take() else {
            return;
        };
        // In the middle of an attempt to join again, waiting could take
        // as long as the network takes to fail. The thread sees the stop
        // between the attempt's steps, ends what it started, and exits on
        // its own: leaving, switching rooms or quitting never waits for it.
        if !self.attempting.load(Ordering::Relaxed) {
            let _ = thread.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop_thread();
    }
}

/// What the session thread holds: the call when one runs, and what it
/// takes to start the next one.
struct SessionThread {
    request: JoinRequest,
    devices: Devices,
    inbox: Receiver<Command>,
    view: Arc<Mutex<CallView>>,
    controls: Arc<Mutex<CallControls>>,
    gate_moved: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    attempting: Arc<AtomicBool>,
    /// The space's event stream, kept from the lost call for the next
    /// one: one subscription, however many attempts it takes.
    events: Option<Receiver<Event>>,
    /// `None` while reconnecting, and after a reconnection failed for good.
    live: Option<Live>,
    retry: Option<Retry>,
}

/// The next attempt to join again.
struct Retry {
    at: Instant,
    /// The pause before the attempt after this one, if this one fails.
    then: Duration,
}

impl Retry {
    /// The first attempt of a sequence, `pause` from now.
    fn first(pause: Duration) -> Retry {
        Retry {
            at: Instant::now() + pause,
            then: (pause * 2).min(RECONNECT_MAX),
        }
    }

    /// The attempt after this one failed: the sequence goes on.
    fn next(&self) -> Retry {
        Retry::first(self.then)
    }

    /// The person asked for the attempt now: it starts at the next tick,
    /// and the sequence starts over from it, so a network that is coming
    /// back is tried again a second later rather than in 15.
    fn now(&mut self) {
        self.at = Instant::now();
        self.then = RECONNECT_FIRST;
    }
}

impl SessionThread {
    /// One tick: the call's events and counters, or the next attempt to
    /// join again.
    fn tick(&mut self) {
        if let Some(live) = self.live.as_mut() {
            live.tick(&self.inbox);
            live.refresh(&self.view);
            if let Some(why) = live.lost.take() {
                self.lose(why);
            }
            return;
        }
        self.read_commands(true);
        if self.retry.as_ref().is_some_and(|r| Instant::now() >= r.at) {
            self.reconnect();
            // A click made during the attempt asked for what just
            // happened: it must not start a second one at once.
            self.read_commands(false);
        }
        if self.live.is_none() {
            self.show_countdown();
        }
    }

    /// Reads the commands sent while no call runs. Volumes chosen meanwhile
    /// apply to the next call; a request to retry now only counts when
    /// `retry` is true, that is not when it came during an attempt.
    fn read_commands(&mut self, retry: bool) {
        while let Ok(command) = self.inbox.try_recv() {
            match command {
                Command::Volume(public_key, volume) => {
                    self.request.settings.volumes.insert(public_key, volume);
                }
                Command::RetryNow if retry => {
                    if let Some(next) = self.retry.as_mut() {
                        next.now();
                    }
                }
                Command::RetryNow => {}
            }
        }
    }

    /// Shows how long until the next attempt, for the interface's
    /// countdown.
    fn show_countdown(&self) {
        let seconds = self
            .retry
            .as_ref()
            .map(|r| seconds_until(r.at, Instant::now()));
        self.view.lock().expect("not poisoned").retry_in_s = seconds;
    }

    /// The server forgot this device's session: stops the call, keeps
    /// what the next one needs, and schedules the first attempt.
    fn lose(&mut self, why: String) {
        let Some(live) = self.live.take() else {
            return;
        };
        let controls = live.controls.clone();
        self.request.settings.volumes = live.volumes.clone();
        if live.view.gate_chosen || self.gate_moved.load(Ordering::Relaxed) {
            self.request.settings.gate_open_dbfs = Some(controls.gate().0);
        }
        let mut view = live.view.clone();
        // The session is already gone: nothing to tell the server.
        self.events = Some(live.end(false));
        view.reconnecting = true;
        view.bound = false;
        view.rtt_ms = None;
        view.input_dbfs = METER_FLOOR_DBFS;
        for person in &mut view.participants {
            person.speaking = false;
        }
        // Nothing is heard until the next call verifies it again.
        view.encryption.state = LockState::Pending;
        for person in &mut view.encryption.people {
            person.verified = false;
            person.rejected = false;
        }
        log_reconnection(&why);
        *self.view.lock().expect("not poisoned") = view;
        self.retry = Some(Retry::first(RECONNECT_FIRST));
        self.show_countdown();
    }

    /// One attempt to join the room again.
    fn reconnect(&mut self) {
        let Some(retry) = self.retry.take() else {
            return;
        };
        // No countdown while the attempt runs: it may take seconds.
        self.show_countdown();
        let muted = self.controls.lock().expect("not poisoned").is_muted();
        self.attempting.store(true, Ordering::Relaxed);
        let attempt = Live::start(
            self.request.clone(),
            (self.devices)(),
            &self.stop,
            muted,
            &mut self.events,
        );
        self.attempting.store(false, Ordering::Relaxed);
        match attempt {
            Ok(live) => {
                // Mute and the gate as they are now, not as they were when
                // the attempt began: the person may have changed them
                // meanwhile, on the old call's controls. Under the lock, so
                // a change lands either before this copy or on the new
                // controls.
                let mut controls = self.controls.lock().expect("not poisoned");
                let gate = controls.gate();
                live.controls.set_muted(controls.is_muted());
                live.controls.set_gate(gate.0, gate.1);
                *controls = live.controls.clone();
                drop(controls);
                self.live = Some(live);
                self.publish();
            }
            Err(e) if e.code == "stopped" => {}
            Err(e) if e.is_temporary() => {
                self.retry = Some(retry.next());
            }
            Err(e) => {
                let mut view = self.view.lock().expect("not poisoned");
                view.reconnecting = false;
                view.ended = Some(e.message);
            }
        }
    }

    /// Shows the running call.
    fn publish(&mut self) {
        if let Some(live) = self.live.as_mut() {
            live.refresh(&self.view);
        }
    }
}

/// Whole seconds from `now` until `at`, rounded up, so the countdown
/// never shows 0 before the attempt starts.
fn seconds_until(at: Instant, now: Instant) -> u32 {
    let left = at.saturating_duration_since(now);
    u32::try_from(left.as_millis().div_ceil(1000)).unwrap_or(u32::MAX)
}

/// Why the call was lost goes to the app's log: never an address, a key
/// or a name.
fn log_reconnection(why: &str) {
    eprintln!("drywire: reconnecting: {why}");
}

/// How a call ends, when an event ends it.
enum End {
    /// The server ended this session on purpose: the call is over.
    Final(String),
    /// The server forgot this session: join again.
    Lost(String),
}

/// Someone the session knows is in the room.
struct Known {
    name: String,
    public_key: String,
}

/// What the session thread holds while the call runs.
struct Live {
    client: Arc<Client>,
    call: Call,
    members: CallMembers,
    controls: CallControls,
    /// Set when the server forgot this session: the session thread then
    /// joins again.
    lost: Option<String>,
    /// Each person's volume other than 1, by public key.
    volumes: BTreeMap<String, f32>,
    notices: Notices,
    /// The counters the loss figures started from: played and concealed,
    /// by session, and the figures of the last window.
    quality: HashMap<u32, (u64, u64, Option<f64>)>,
    ticks: u32,
    /// `None` once the server ended the call and the sound card stopped.
    device: Option<Box<dyn AudioIo>>,
    events: Receiver<Event>,
    snapshots: u32,
    me: u32,
    room_id: Id,
    people: HashMap<u32, Known>,
    view: CallView,
}

impl Live {
    /// Joins the room and starts the sound card. `muted` applies before
    /// the microphone opens. `events` is the space's event stream, opened
    /// here when there is none yet, and taken only once the call runs, so
    /// a failed attempt leaves it for the next. Between its steps it gives
    /// up, with the code `stopped`, once `stop` is set.
    fn start(
        request: JoinRequest,
        device: DeviceFactory,
        stop: &AtomicBool,
        muted: bool,
        events: &mut Option<Receiver<Event>>,
    ) -> Result<Live, AppError> {
        let check = || {
            if stop.load(Ordering::Relaxed) {
                Err(AppError::new("stopped", "the call was left"))
            } else {
                Ok(())
            }
        };
        let client = request.client;
        let space = client.space(&request.space_id)?;
        let room = space
            .rooms
            .iter()
            .find(|r| r.id == request.room_id)
            .cloned()
            .ok_or_else(|| AppError::new("not_found", "this room no longer exists"))?;
        // The event stream before the session, so no arrival is missed. A
        // new stream begins with a snapshot older than the join answer,
        // which is skipped; one kept from an earlier call has none pending.
        let fresh_stream = events.is_none();
        if fresh_stream {
            check()?;
            *events = Some(follow(&client, &request.space_id)?);
        }
        check()?;
        let joined = client.join(&room.id)?;
        let relay = resolve(&joined.relay)?;
        let me = joined.session;

        let mut people = HashMap::new();
        let mut members = Vec::new();
        for p in joined.sessions.iter().filter(|p| p.room_id == room.id) {
            members.push(Member {
                session: p.session,
                name: printable(&p.name),
                public_key: public_key(p)?,
            });
            people.insert(p.session, known(p));
        }
        let config = ClientConfig {
            room_id: room.id.0,
            space_key: request.space_key,
            session: me,
            session_secret: joined.session_secret.0,
            identity: request.identity,
            members,
        };
        // The defaults of `drywire join`: 5 ms frames at 64 kb/s.
        let gate = gate_config(
            request
                .settings
                .gate_open_dbfs
                .unwrap_or(GateConfig::default().open_dbfs),
        );
        let options = CallOptions {
            relay,
            sender: SenderConfig {
                gate,
                ..SenderConfig::new(FrameSize::Ms5)
            },
            link: LinkConfig::default(),
            join_timeout: Duration::from_secs(5),
            min_buffer_us: 0,
        };
        check()?;
        let (mut call, capture, playback) =
            Call::join(&config, options).map_err(|e| AppError::new("relay", e.to_string()))?;
        let controls = call.controls();
        controls.set_muted(muted);
        if let Err(e) = check() {
            // The sound card never ran: its callbacks go first, then the
            // call and the session.
            drop((capture, playback));
            call.leave();
            let _ = client.leave();
            return Err(e);
        }
        let mut device = device().map_err(|e| AppError::new("audio", e))?;
        let streams = device
            .start(capture, playback)
            .map_err(|e| AppError::new("audio", e.to_string()))?;
        let members = call.members();
        let volumes = request.settings.volumes;
        for (session, person) in &people {
            if let Some(&volume) = volumes.get(&person.public_key) {
                members.set_volume(*session, volume);
            }
        }
        let instance = client.instance().to_string();
        Ok(Live {
            client,
            call,
            members,
            controls,
            lost: None,
            volumes,
            notices: request.notices,
            quality: HashMap::new(),
            ticks: 0,
            device: Some(device),
            events: events.take().expect("opened above"),
            snapshots: if fresh_stream { 0 } else { 1 },
            me,
            room_id: room.id,
            people,
            view: CallView {
                instance,
                space_id: request.space_id.to_hex(),
                space_name: printable(&request.space_name),
                room_id: room.id.to_hex(),
                room_name: printable(&room.name),
                devices: streams.iter().map(DeviceView::new).collect(),
                gate_dbfs: gate.open_dbfs,
                gate_chosen: request.settings.gate_open_dbfs.is_some(),
                input_dbfs: METER_FLOOR_DBFS,
                device_ms: request.settings.device_loopback_ms.unwrap_or_else(|| {
                    streams.iter().map(|s| s.buffer_ms() + s.latency_ms()).sum()
                }),
                device_measured: request.settings.device_loopback_ms.is_some(),
                ..CallView::default()
            },
        })
    }

    /// Applies the commands and events that arrived since the last tick.
    fn tick(&mut self, commands: &Receiver<Command>) {
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Volume(public_key, volume) => {
                    for (session, person) in &self.people {
                        if person.public_key == public_key {
                            self.members.set_volume(*session, volume);
                        }
                    }
                    self.volumes.insert(public_key, volume);
                }
                // A click that reached a call already running again.
                Command::RetryNow => {}
            }
        }
        while let Ok(event) = self.events.try_recv() {
            // The first snapshot was taken before this device joined; the
            // join answer is newer. Later ones follow a reconnection.
            if matches!(event, Event::Snapshot(_)) {
                self.snapshots += 1;
                if self.snapshots == 1 {
                    continue;
                }
            }
            if self.view.ended.is_some() || self.lost.is_some() {
                continue;
            }
            match self.on_event(event) {
                Some(End::Final(why)) => {
                    self.view.ended = Some(why);
                    self.stop_audio();
                }
                Some(End::Lost(why)) => self.lost = Some(why),
                None => {}
            }
        }
    }

    /// Applies one event. Returns how the call ends, if this device's own
    /// session ended.
    fn on_event(&mut self, event: Event) -> Option<End> {
        match event {
            Event::Join(p) => self.add(&p),
            Event::Snapshot(everyone) => {
                // After the stream was opened again: whoever came meanwhile
                // is added, whoever left is removed.
                for p in &everyone {
                    self.add(p);
                }
                let present: Vec<u32> = everyone
                    .iter()
                    .filter(|p| p.room_id == self.room_id)
                    .map(|p| p.session)
                    .collect();
                if !present.contains(&self.me) {
                    return Some(End::Lost(
                        "this session ended while the event stream was down".into(),
                    ));
                }
                let (me, members, notices) = (self.me, &self.members, &self.notices);
                self.people.retain(|session, person| {
                    let keep = *session == me || present.contains(session);
                    if !keep {
                        members.remove(*session);
                        notices(Notice {
                            kind: "leave",
                            name: person.name.clone(),
                        });
                    }
                    keep
                });
            }
            // Joined elsewhere, or removed: on purpose.
            Event::Leave(p) if p.session == self.me => {
                return Some(End::Final(
                    "the server ended this session: joined elsewhere, or removed".into(),
                ));
            }
            // The relay stopped hearing from this device.
            Event::Disconnect(p) if p.session == self.me => {
                return Some(End::Lost(
                    "the relay stopped hearing from this device".into(),
                ));
            }
            Event::Leave(p) => self.remove(p.session, "leave"),
            Event::Disconnect(p) => self.remove(p.session, "disconnect"),
            Event::Other(_) => {}
        }
        None
    }

    fn add(&mut self, p: &Presence) {
        if p.room_id != self.room_id || p.session == self.me || self.people.contains_key(&p.session)
        {
            return;
        }
        if let Ok(key) = public_key(p) {
            self.members.add(p.session, key);
            let person = known(p);
            if let Some(&volume) = self.volumes.get(&person.public_key) {
                self.members.set_volume(p.session, volume);
            }
            (self.notices)(Notice {
                kind: "join",
                name: person.name.clone(),
            });
            self.people.insert(p.session, person);
        }
    }

    /// Someone left the room, or the relay stopped hearing from them.
    fn remove(&mut self, session: u32, kind: &'static str) {
        if let Some(person) = self.people.remove(&session) {
            self.members.remove(session);
            (self.notices)(Notice {
                kind,
                name: person.name,
            });
        }
    }

    /// Copies the call's counters into the shared view.
    fn refresh(&mut self, shared: &Mutex<CallView>) {
        let stats = self.call.stats();
        self.view.bound = stats.link.bound && self.view.ended.is_none();
        self.view.rtt_ms = stats.link.rtt_us.map(|us| us as f64 / 1000.0);
        self.view.min_rtt_ms = stats.link.min_rtt_us.map(|us| us as f64 / 1000.0);
        self.view.muted = self.controls.is_muted();
        self.view.gate_dbfs = self.controls.gate().0;
        let peak = self.controls.take_input_peak_dbfs().max(METER_FLOOR_DBFS);
        self.view.input_dbfs = peak.max(self.view.input_dbfs - METER_FALL_DB);
        let link = &stats.link;
        let expected = u64::from(link.uplink_received) + u64::from(link.uplink_lost);
        self.view.uplink_loss_pct = if expected == 0 {
            0.0
        } else {
            100.0 * f64::from(link.uplink_lost) / expected as f64
        };
        self.ticks += 1;
        if self.ticks.is_multiple_of(QUALITY_TICKS) {
            self.measure_loss(&stats);
        }
        self.view.participants = self.participants(&stats);
        self.view.encryption = self.encryption(&stats);
        *shared.lock().expect("not poisoned") = self.view.clone();
    }

    /// Updates each sender's loss over the last window.
    fn measure_loss(&mut self, stats: &CallStats) {
        let mut next = HashMap::new();
        for track in &stats.tracks {
            let (played, concealed) = (track.jitter.played, track.jitter.concealed);
            let (was_played, was_concealed, _) = self
                .quality
                .get(&track.session)
                .copied()
                .unwrap_or((0, 0, None));
            let ticks = (played + concealed).saturating_sub(was_played + was_concealed);
            let loss = (ticks > 0)
                .then(|| 100.0 * concealed.saturating_sub(was_concealed) as f64 / ticks as f64);
            next.insert(track.session, (played, concealed, loss));
        }
        self.quality = next;
    }

    /// Everyone in the room, this device first, then by name.
    fn participants(&self, stats: &CallStats) -> Vec<Participant> {
        let mut list: Vec<Participant> = self
            .people
            .iter()
            .map(|(&session, k)| {
                let track = stats.tracks.iter().find(|t| t.session == session);
                let me = session == self.me;
                let frame_ms = track
                    .and_then(|t| t.frame)
                    .map(|f| f.samples() as f64 * 1000.0 / f64::from(SAMPLE_RATE));
                let buffer_ms = frame_ms
                    .and(track)
                    .map(|t| t.jitter.target_us as f64 / 1000.0);
                let estimate_ms = match (frame_ms, buffer_ms, self.view.rtt_ms) {
                    (Some(frame), Some(buffer), Some(rtt)) => {
                        Some(estimate_ms(self.view.device_ms, frame, rtt, buffer))
                    }
                    _ => None,
                };
                Participant {
                    session,
                    name: k.name.clone(),
                    public_key: k.public_key.clone(),
                    me,
                    speaking: if me {
                        stats.talking
                    } else {
                        track.is_some_and(|t| t.speaking)
                    },
                    volume: self.volumes.get(&k.public_key).copied().unwrap_or(1.0),
                    buffer_ms,
                    loss_pct: self.quality.get(&session).and_then(|q| q.2),
                    estimate_ms,
                    counters: track.map_or_else(Counters::default, |t| Counters {
                        received: t.jitter.received,
                        played: t.jitter.played,
                        concealed: t.jitter.concealed,
                        late: t.jitter.late,
                        dropped: t.jitter.dropped,
                        depth: t.depth,
                    }),
                }
            })
            .collect();
        list.sort_by(|a, b| {
            b.me.cmp(&a.me)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then(a.session.cmp(&b.session))
        });
        list
    }

    /// The lock's state and each person's, in the participants' order.
    fn encryption(&self, stats: &CallStats) -> EncryptionView {
        let senders = &stats.encryption.senders;
        EncryptionView {
            state: stats.encryption.state.into(),
            people: self
                .view
                .participants
                .iter()
                .filter(|p| !p.me)
                .map(|p| {
                    let state = senders
                        .iter()
                        .find(|s| s.session == p.session)
                        .map(|s| s.state);
                    PersonLock {
                        public_key: p.public_key.clone(),
                        verified: state == Some(SenderState::Verified),
                        rejected: state == Some(SenderState::Failed),
                    }
                })
                .collect(),
        }
    }

    fn stop_audio(&mut self) {
        if let Some(mut device) = self.device.take() {
            device.stop();
        }
    }

    /// Stops the sound card first, then the call, as the core requires.
    /// Returns the space's event stream, which a reconnection reuses.
    fn end(mut self, tell_server: bool) -> Receiver<Event> {
        self.stop_audio();
        self.call.leave();
        if tell_server && self.view.ended.is_none() {
            let _ = self.client.leave();
        }
        self.events
    }
}

fn known(p: &Presence) -> Known {
    Known {
        name: printable(&p.name),
        public_key: to_hex(&p.public_key.0),
    }
}

fn resolve(relay: &str) -> Result<SocketAddr, AppError> {
    relay
        .to_socket_addrs()
        .map_err(|e| AppError::new("relay", format!("relay {relay}: {e}")))?
        .next()
        .ok_or_else(|| AppError::new("relay", format!("relay {relay}: no address")))
}

fn public_key(p: &Presence) -> Result<VerifyingKey, AppError> {
    VerifyingKey::from_bytes(&p.public_key.0).map_err(|_| {
        AppError::new(
            "protocol",
            format!("the server sent an invalid key for session {}", p.session),
        )
    })
}

/// Reads the space's events on a thread of its own, opening the stream
/// again whenever it ends, until the receiver is dropped. A stream ends
/// at its first error, a silent one included (`drywire_control::Events`),
/// and the next begins with a snapshot that puts the call right.
fn follow(client: &Arc<Client>, space: &Id) -> Result<Receiver<Event>, AppError> {
    let first = client.events(space)?;
    let (tx, rx) = mpsc::channel();
    let (client, space) = (Arc::clone(client), *space);
    std::thread::Builder::new()
        .name("drywire-events".into())
        .spawn(move || {
            let mut stream = Some(first);
            loop {
                if let Some(events) = stream.take() {
                    for event in events.flatten() {
                        if tx.send(event).is_err() {
                            return;
                        }
                    }
                }
                std::thread::sleep(EVENTS_RETRY);
                match client.events(&space) {
                    Ok(events) => stream = Some(events),
                    // No longer a member, or the space is gone: stop.
                    Err(e) if e.status() == Some(404) => return,
                    Err(_) => {}
                }
            }
        })
        .map_err(|e| AppError::new("network", e.to_string()))?;
    Ok(rx)
}

/// A name from a server, safe to show: control characters become `?`. An
/// honest server never sends them (protocol section 13.4).
pub fn printable(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_countdown_rounds_up() {
        let now = Instant::now();
        assert_eq!(seconds_until(now + Duration::from_millis(14_001), now), 15);
        assert_eq!(seconds_until(now + Duration::from_secs(1), now), 1);
        assert_eq!(seconds_until(now + Duration::from_millis(1), now), 1);
        assert_eq!(
            seconds_until(now, now + Duration::from_secs(1)),
            0,
            "overdue"
        );
        let json = serde_json::to_value(CallView {
            retry_in_s: Some(4),
            ..CallView::default()
        })
        .unwrap();
        assert_eq!(json["retryInS"], 4);
    }

    #[test]
    fn retrying_now_starts_the_sequence_over() {
        let first = Retry::first(RECONNECT_FIRST);
        let pauses: Vec<u64> = std::iter::successors(Some(first), |r| Some(r.next()))
            .take(7)
            .map(|r| r.then.as_secs())
            .collect();
        assert_eq!(pauses, [2, 4, 8, 15, 15, 15, 15], "ADR 0035's pauses");

        let mut late = Retry::first(RECONNECT_MAX);
        late.now();
        assert!(late.at <= Instant::now(), "the attempt is due at once");
        let after = late.next();
        assert_eq!(
            seconds_until(after.at, Instant::now()),
            1,
            "a failed manual attempt is tried again a second later"
        );
        assert_eq!(after.then, Duration::from_secs(2), "then 2, 4, 8, 15");
    }

    #[test]
    fn the_lock_reaches_the_interface_in_camel_case() {
        let view = EncryptionView {
            state: LockState::Verified,
            people: vec![PersonLock {
                public_key: "ab".into(),
                verified: true,
                rejected: false,
            }],
        };
        assert_eq!(
            serde_json::to_string(&view).unwrap(),
            r#"{"state":"verified","people":[{"publicKey":"ab","verified":true,"rejected":false}]}"#
        );
        let json = serde_json::to_value(CallView::default()).unwrap();
        assert_eq!(json["encryption"]["state"], "pending");
    }
}
