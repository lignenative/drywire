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
use std::time::Duration;

use drywire_control::{Client, Event, Id, Presence, to_hex};
use drywire_core::audio::{AudioIo, StreamInfo};
use drywire_core::call::{Call, CallControls, CallMembers, CallOptions, CallStats};
use drywire_core::config::{ClientConfig, Member};
use drywire_core::gate::GateConfig;
use drywire_core::link::LinkConfig;
use drywire_core::opus::FrameSize;
use drywire_core::sender::SenderConfig;
use drywire_proto::crypto::{SigningKey, VerifyingKey};
use serde::Serialize;

use crate::error::AppError;
use crate::store::Settings;

/// Opens the sound card, on the session thread.
pub type DeviceFactory = Box<dyn FnOnce() -> Result<Box<dyn AudioIo>, String> + Send>;

/// How often the session thread wakes up: to apply the space's events and
/// refresh the [`CallView`].
const TICK: Duration = Duration::from_millis(20);

/// How long to wait before opening the event stream again after it ends.
const EVENTS_RETRY: Duration = Duration::from_secs(2);

/// How far below the open threshold the gate closes: the core's default
/// hysteresis, so one setting moves both.
pub const GATE_HYSTERESIS_DB: f32 = 6.0;

/// The bottom of the level meter, in dBFS: quieter shows as empty.
pub const METER_FLOOR_DBFS: f32 = -90.0;

/// How fast the meter falls, per tick: 50 dB a second, so a word's peak
/// stays visible between two polls of the interface.
const METER_FALL_DB: f32 = 1.0;

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
}

/// What the interface asks of the session thread.
enum Command {
    /// Play everyone with this public key (hex) at this volume.
    Volume(String, f32),
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
    fn new(info: &StreamInfo) -> DeviceView {
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
    /// Whether the relay acknowledged this device and still answers.
    pub bound: bool,
    /// The last round trip to the relay, in milliseconds.
    pub rtt_ms: Option<f64>,
    /// Whether the microphone is muted.
    pub muted: bool,
    /// The microphone's level for the meter, in dBFS, from
    /// [`METER_FLOOR_DBFS`] up: it rises at once and falls slowly.
    pub input_dbfs: f32,
    /// The level at which the voice gate opens, in dBFS.
    pub gate_dbfs: f32,
    /// Our packets the relay expected and never got, over the call, in
    /// percent.
    pub uplink_loss_pct: f64,
    /// Everyone in the room, this device first.
    pub participants: Vec<Participant>,
    /// The audio streams, as the system opened them.
    pub devices: Vec<DeviceView>,
}

/// A running session. Dropping it ends the call.
pub struct Session {
    view: Arc<Mutex<CallView>>,
    controls: CallControls,
    commands: Sender<Command>,
    stop: Arc<AtomicBool>,
    /// Whether ending should also tell the control plane: not when
    /// switching rooms, where the next join replaces the session anyway.
    leave_server: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    /// Joins the room and starts the sound card that `device` opens.
    /// Returns once audio runs, or with why it could not start.
    pub fn start(request: JoinRequest, device: DeviceFactory) -> Result<Session, AppError> {
        let view = Arc::new(Mutex::new(CallView::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let leave_server = Arc::new(AtomicBool::new(true));
        let (ready, started) = mpsc::channel();
        let (commands, inbox) = mpsc::channel();
        let thread = {
            let (view, stop, leave_server) = (
                Arc::clone(&view),
                Arc::clone(&stop),
                Arc::clone(&leave_server),
            );
            std::thread::Builder::new()
                .name("drywire-session".into())
                .spawn(move || {
                    let mut live = match Live::start(request, device, inbox) {
                        Ok(live) => live,
                        Err(e) => {
                            let _ = ready.send(Err(e));
                            return;
                        }
                    };
                    live.refresh(&view);
                    let _ = ready.send(Ok(live.call.controls()));
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(TICK);
                        live.tick();
                        live.refresh(&view);
                    }
                    live.end(leave_server.load(Ordering::Relaxed));
                })
                .map_err(|e| AppError::new("audio", e.to_string()))?
        };
        match started.recv() {
            Ok(Ok(controls)) => Ok(Session {
                view,
                controls,
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
        self.view.lock().expect("not poisoned").clone()
    }

    /// Mutes or unmutes the microphone, at the capture callback's next
    /// call.
    pub fn set_muted(&self, muted: bool) {
        self.controls.set_muted(muted);
        self.view.lock().expect("not poisoned").muted = muted;
    }

    /// Moves the voice gate's threshold (clamped to −80 through −10 dBFS).
    pub fn set_gate(&self, open_dbfs: f32) {
        let gate = gate_config(open_dbfs);
        self.controls.set_gate(gate.open_dbfs, gate.close_dbfs);
        self.view.lock().expect("not poisoned").gate_dbfs = gate.open_dbfs;
    }

    /// Plays the person with this public key (hex) at `volume`, within a
    /// tick.
    pub fn set_volume(&self, public_key: &str, volume: f32) {
        let _ = self
            .commands
            .send(Command::Volume(public_key.to_string(), volume));
    }

    /// Ends the call. With `tell_server`, the control plane frees the
    /// room at once rather than after the relay's timeout.
    pub fn end(mut self, tell_server: bool) {
        self.leave_server.store(tell_server, Ordering::Relaxed);
        self.stop_thread();
    }

    fn stop_thread(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop_thread();
    }
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
    commands: Receiver<Command>,
    /// Each person's volume other than 1, by public key.
    volumes: BTreeMap<String, f32>,
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
    fn start(
        request: JoinRequest,
        device: DeviceFactory,
        commands: Receiver<Command>,
    ) -> Result<Live, AppError> {
        let client = request.client;
        let space = client.space(&request.space_id)?;
        let room = space
            .rooms
            .iter()
            .find(|r| r.id == request.room_id)
            .cloned()
            .ok_or_else(|| AppError::new("not_found", "this room no longer exists"))?;
        // The event stream before the session, so no arrival is missed.
        let events = follow(&client, &request.space_id)?;
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
        let gate = gate_config(request.settings.gate_open_dbfs);
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
        let (call, capture, playback) =
            Call::join(&config, options).map_err(|e| AppError::new("relay", e.to_string()))?;
        let mut device = device().map_err(|e| AppError::new("audio", e))?;
        let streams = device
            .start(capture, playback)
            .map_err(|e| AppError::new("audio", e.to_string()))?;
        let members = call.members();
        let controls = call.controls();
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
            commands,
            volumes,
            quality: HashMap::new(),
            ticks: 0,
            device: Some(device),
            events,
            snapshots: 0,
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
                input_dbfs: METER_FLOOR_DBFS,
                ..CallView::default()
            },
        })
    }

    /// Applies the commands and events that arrived since the last tick.
    fn tick(&mut self) {
        while let Ok(command) = self.commands.try_recv() {
            match command {
                Command::Volume(public_key, volume) => {
                    for (session, person) in &self.people {
                        if person.public_key == public_key {
                            self.members.set_volume(*session, volume);
                        }
                    }
                    self.volumes.insert(public_key, volume);
                }
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
            if let Some(why) = self.on_event(event)
                && self.view.ended.is_none()
            {
                self.view.ended = Some(why);
                self.stop_audio();
            }
        }
    }

    /// Applies one event. Returns why the call must end, if it must: this
    /// device's own session ended.
    fn on_event(&mut self, event: Event) -> Option<String> {
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
                    return Some("this session ended while the event stream was down".into());
                }
                let (me, members) = (self.me, &self.members);
                self.people.retain(|session, _| {
                    let keep = *session == me || present.contains(session);
                    if !keep {
                        members.remove(*session);
                    }
                    keep
                });
            }
            Event::Leave(p) | Event::Disconnect(p) if p.session == self.me => {
                return Some(
                    "the server ended this session: joined elsewhere, removed, or silent too long"
                        .into(),
                );
            }
            Event::Leave(p) | Event::Disconnect(p) => {
                if self.people.remove(&p.session).is_some() {
                    self.members.remove(p.session);
                }
            }
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
            self.people.insert(p.session, person);
        }
    }

    /// Copies the call's counters into the shared view.
    fn refresh(&mut self, shared: &Mutex<CallView>) {
        let stats = self.call.stats();
        self.view.bound = stats.link.bound && self.view.ended.is_none();
        self.view.rtt_ms = stats.link.rtt_us.map(|us| us as f64 / 1000.0);
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
                    buffer_ms: track
                        .filter(|t| t.frame.is_some())
                        .map(|t| t.jitter.target_us as f64 / 1000.0),
                    loss_pct: self.quality.get(&session).and_then(|q| q.2),
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

    fn stop_audio(&mut self) {
        if let Some(mut device) = self.device.take() {
            device.stop();
        }
    }

    /// Stops the sound card first, then the call, as the core requires.
    fn end(mut self, tell_server: bool) {
        self.stop_audio();
        self.call.leave();
        if tell_server && self.view.ended.is_none() {
            let _ = self.client.leave();
        }
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
/// again whenever it ends, until the receiver is dropped.
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
