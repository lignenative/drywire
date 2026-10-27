//! A call: the pieces of this crate wired to a UDP socket and to the audio
//! callbacks (ADR 0014).
//!
//! Three threads share the work, so that nothing waits on anything slow:
//!
//! - **Capture** (the audio input callback): frames the microphone's
//!   samples, runs the [`Sender`], and sends its packets on the socket at
//!   once. Sending from here saves a thread hop on the path every word
//!   takes. The socket is non-blocking: a packet the OS cannot take at
//!   once is dropped and counted, never waited for (ADR 0016).
//! - **Network** (a thread of its own): receives, checks and decrypts with
//!   [`Inbound`], drives the [`Link`] (pings, keepalives, new hellos), and
//!   passes decrypted frames to playback through a wait-free ring.
//! - **Playback** (the audio output callback): drains that ring into the
//!   [`Mixer`] and renders the mix.
//!
//! The audio callbacks never wait for a lock: they update the shared
//! statistics only when the lock is free. Once the call has started they
//! never allocate or free memory either (ADR 0017): every buffer they use
//! is made in [`Call::join`], or, for a member who joins later
//! ([`Call::add_member`]), by the network thread, which hands playback a
//! ready-made track.

use std::collections::VecDeque;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender as ChangeSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use drywire_proto::crypto::VerifyingKey;
use drywire_proto::header::MAX_PACKET_SIZE;
use rtrb::{Consumer, Producer, RingBuffer};

use crate::audio::{InputCallback, OutputCallback};
use crate::config::ClientConfig;
use crate::inbound::{Event, Inbound, InboundStats};
use crate::jitter::JitterStats;
use crate::link::{Link, LinkConfig, LinkStats};
use crate::mixer::{Mixer, Slot, TrackStats};
use crate::opus::OpusError;
use crate::sender::{Packets, Sender, SenderConfig, SenderStats};
use crate::socket;

/// How a call is set up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CallOptions {
    /// The relay's UDP address.
    pub relay: SocketAddr,
    /// How this client encodes and sends.
    pub sender: SenderConfig,
    /// When the link pings.
    pub link: LinkConfig,
    /// How long to wait for the relay to acknowledge the first `hello`.
    pub join_timeout: Duration,
    /// The smallest delay the jitter buffers aim for, in microseconds; it
    /// raises their one-frame floor when larger (see
    /// [`Mixer::with_min_target`]).
    pub min_buffer_us: u64,
}

/// Everything a call counts, for display.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallStats {
    /// The relay link: binding, round trip, uplink loss.
    pub link: LinkStats,
    /// What this client sent.
    pub sender: SenderStats,
    /// Whether the voice gate is open.
    pub talking: bool,
    /// Packets dropped because the socket's send buffer was full: the
    /// capture callback never waits for room (ADR 0016).
    pub send_dropped: u64,
    /// Packets the socket refused to send for any other reason.
    pub send_errors: u64,
    /// What was received and dropped before playback.
    pub inbound: InboundStats,
    /// Frames dropped because playback had not taken the previous ones yet
    /// (see [`PLAYBACK_QUEUE`]): playback stalled.
    pub playback_queue_full: u64,
    /// Each sender's playback.
    pub tracks: Vec<TrackStats>,
    /// The playback counters of the senders who left the room, summed.
    pub departed: JitterStats,
}

/// Why a call could not start.
#[derive(Debug)]
pub enum CallError {
    /// The socket could not be opened.
    Io(std::io::Error),
    /// The relay did not answer the `hello` in time.
    NoAnswer,
    /// The Opus encoder could not be made.
    Opus(OpusError),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Io(e) => write!(f, "network: {e}"),
            CallError::NoAnswer => f.write_str(
                "the relay did not answer: check that its address is reachable (UDP) and \
                 that the session is still valid; join the room again for a new one",
            ),
            CallError::Opus(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for CallError {}

impl From<std::io::Error> for CallError {
    fn from(e: std::io::Error) -> CallError {
        CallError::Io(e)
    }
}

/// How many decrypted frames may wait for the playback callback. It drains
/// them every callback, every few milliseconds; 256 frames are more than a
/// 60 ms stall of ten senders at 2.5 ms frames.
pub const PLAYBACK_QUEUE: usize = 256;

/// The most tracks playback holds: more than the other members a room can
/// have (32 at the reference relay), so a call never runs out.
pub const MAX_TRACKS: usize = 64;

/// What the network thread passes to playback, through a wait-free ring.
#[expect(
    clippy::large_enum_variant,
    reason = "frames are inline on purpose: boxing them would allocate (ADR 0017)"
)]
enum ToPlayback {
    /// A decrypted frame, or a sender's change, stamped with its arrival.
    Event(u64, Event),
    /// A spare track for a member who just joined.
    Slot(Slot),
}

/// Changes the members of a running call; cheap to clone, and usable from
/// any thread.
#[derive(Clone)]
pub struct CallMembers(ChangeSender<Change>);

impl CallMembers {
    /// Someone joined the room: accept their announcements, verified with
    /// `public_key`, and play their audio. Takes effect within the network
    /// thread's next wake-up (50 ms at most). This client's own session is
    /// ignored, as is a session already known.
    pub fn add(&self, session: u32, public_key: VerifyingKey) {
        let _ = self.0.send(Change::Add(session, public_key));
    }

    /// Someone left the room: forget their keys, and free their track for
    /// the next member.
    pub fn remove(&self, session: u32) {
        let _ = self.0.send(Change::Remove(session));
    }
}

/// A change of the room's members, from the control plane.
enum Change {
    Add(u32, VerifyingKey),
    Remove(u32),
}

/// State shared by the three threads.
struct Shared {
    started: Instant,
    stop: AtomicBool,
    /// Last time the capture thread sent a packet, in microseconds since
    /// `started`, so the link knows when a keepalive is needed.
    last_sent_us: AtomicU64,
    stats: Mutex<CallStats>,
}

impl Shared {
    fn now_us(&self) -> u64 {
        self.started.elapsed().as_micros() as u64
    }
}

/// A call in progress. Dropping it stops the network thread; stop the audio
/// device first, so the callbacks stop too.
pub struct Call {
    shared: Arc<Shared>,
    network: Option<JoinHandle<()>>,
    changes: ChangeSender<Change>,
}

impl Call {
    /// Opens a socket to the relay, says `hello` until it answers, sends the
    /// first announcement, and starts the network thread. Returns the call
    /// and the two callbacks to give to an audio device.
    pub fn join(
        config: &ClientConfig,
        options: CallOptions,
    ) -> Result<(Call, InputCallback, OutputCallback), CallError> {
        let local: SocketAddr = if options.relay.is_ipv4() {
            "0.0.0.0:0".parse().expect("valid")
        } else {
            "[::]:0".parse().expect("valid")
        };
        let socket = UdpSocket::bind(local)?;
        // Only the relay's packets come through a connected socket.
        socket.connect(options.relay)?;
        let socket = Arc::new(socket);

        let others: Vec<_> = config
            .members
            .iter()
            .filter(|m| m.session != config.session)
            .map(|m| (m.session, m.public_key))
            .collect();
        let shared = Arc::new(Shared {
            started: Instant::now(),
            stop: AtomicBool::new(false),
            last_sent_us: AtomicU64::new(0),
            stats: Mutex::new(CallStats {
                // Room for every track, so playback never grows it.
                tracks: Vec::with_capacity(MAX_TRACKS),
                ..CallStats::default()
            }),
        });
        let mut link = Link::new(config.session, config.session_secret, options.link);
        bind(&socket, &shared, &mut link, options.join_timeout)?;
        // From here on nothing waits on the socket itself: the network
        // thread waits with `socket::wait_readable`, and the capture
        // callback drops what the OS cannot take at once.
        socket.set_nonblocking(true)?;

        let mut sender = Sender::new(
            config.identity.clone(),
            config.session,
            config.room_id,
            &config.space_key,
            options.sender,
        )
        .map_err(CallError::Opus)?;
        // The epoch's first announcement, before any audio (section 8.3).
        let _ = socket.send(&sender.announce());

        // Every track before playback starts: making one allocates. Room
        // for the tracks of members who join later, handed in as slots.
        let mut mixer = Mixer::with_min_target(options.min_buffer_us);
        mixer.reserve(MAX_TRACKS);
        for (session, _) in others.iter().take(MAX_TRACKS) {
            mixer.add_track(*session).map_err(CallError::Opus)?;
        }
        let members = Members {
            me: config.session,
            peers: others.len(),
            tracks: others.len().min(MAX_TRACKS),
            changes: None,
            pending: VecDeque::new(),
        };
        let inbound = Inbound::new(config.room_id, &config.space_key, others);
        let (events, playback_events) = RingBuffer::new(PLAYBACK_QUEUE);
        let (changes, changes_rx) = mpsc::channel();

        let network = {
            let socket = Arc::clone(&socket);
            let shared = Arc::clone(&shared);
            let members = Members {
                changes: Some(changes_rx),
                ..members
            };
            std::thread::Builder::new()
                .name("drywire-network".into())
                .spawn(move || network_loop(&socket, &shared, link, inbound, members, events))?
        };
        let input = capture_callback(socket, Arc::clone(&shared), sender);
        let output = playback_callback(Arc::clone(&shared), playback_events, mixer);
        Ok((
            Call {
                shared,
                network: Some(network),
                changes,
            },
            input,
            output,
        ))
    }

    /// Someone joined the room: see [`CallMembers::add`].
    pub fn add_member(&self, session: u32, public_key: VerifyingKey) {
        self.members().add(session, public_key);
    }

    /// Someone left the room: see [`CallMembers::remove`].
    pub fn remove_member(&self, session: u32) {
        self.members().remove(session);
    }

    /// A handle that changes the call's members from another thread, such
    /// as the one reading the control plane's events.
    pub fn members(&self) -> CallMembers {
        CallMembers(self.changes.clone())
    }

    /// A snapshot of the counters.
    pub fn stats(&self) -> CallStats {
        self.shared.stats.lock().expect("not poisoned").clone()
    }

    /// Stops the network thread.
    pub fn leave(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(network) = self.network.take() {
            let _ = network.join();
        }
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        self.leave();
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Says `hello` until the relay acknowledges it, or `timeout` passes.
fn bind(
    socket: &UdpSocket,
    shared: &Shared,
    link: &mut Link,
    timeout: Duration,
) -> Result<(), CallError> {
    socket.set_read_timeout(Some(Duration::from_millis(20)))?;
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; MAX_PACKET_SIZE];
    while !link.is_bound() {
        if Instant::now() >= deadline {
            return Err(CallError::NoAnswer);
        }
        if let Some(packet) = link.poll(shared.now_us(), unix_ms(), 0) {
            // A refused send (no route yet) is retried with the next hello.
            let _ = socket.send(&packet);
        }
        if let Ok(n) = socket.recv(&mut buf) {
            let _ = link.handle(shared.now_us(), &buf[..n]);
        }
    }
    Ok(())
}

/// The network thread's view of the room's members.
struct Members {
    /// This client's session, never a peer.
    me: u32,
    /// How many other members [`Inbound`] accepts packets from.
    peers: usize,
    /// How many tracks playback holds, used or free. Kept at least equal
    /// to `peers`, so a new sender always finds a free track.
    tracks: usize,
    changes: Option<Receiver<Change>>,
    /// Messages for playback that found its ring full, sent first next
    /// time: a lost slot or `Left` would leave playback a track short.
    pending: VecDeque<ToPlayback>,
}

impl Members {
    /// Applies the changes the control plane sent since the last call.
    fn apply(&mut self, now: u64, inbound: &mut Inbound, events: &mut Producer<ToPlayback>) {
        while let Some(message) = self.pending.pop_front() {
            if let Err(rtrb::PushError::Full(message)) = events.push(message) {
                self.pending.push_front(message);
                return;
            }
        }
        let Some(changes) = &self.changes else {
            return;
        };
        while let Ok(change) = changes.try_recv() {
            match change {
                Change::Add(session, key) if session != self.me => {
                    if !inbound.add_peer(session, key) {
                        continue;
                    }
                    self.peers += 1;
                    if self.peers > self.tracks && self.tracks < MAX_TRACKS {
                        // Made here, where allocating is fine.
                        if let Ok(slot) = Mixer::make_slot() {
                            self.tracks += 1;
                            self.pending.push_back(ToPlayback::Slot(slot));
                        }
                    }
                }
                Change::Add(..) => {}
                Change::Remove(session) => {
                    if inbound.remove_peer(session) {
                        self.peers -= 1;
                        let left = Event::Left { session };
                        self.pending.push_back(ToPlayback::Event(now, left));
                    }
                }
            }
        }
        while let Some(message) = self.pending.pop_front() {
            if let Err(rtrb::PushError::Full(message)) = events.push(message) {
                self.pending.push_front(message);
                return;
            }
        }
    }
}

/// Receives, decrypts and forwards to playback; drives the link.
fn network_loop(
    socket: &UdpSocket,
    shared: &Shared,
    mut link: Link,
    mut inbound: Inbound,
    mut members: Members,
    mut events: Producer<ToPlayback>,
) {
    let mut buf = [0u8; MAX_PACKET_SIZE];
    let mut next_stats_us = 0;
    let mut queue_full = 0;
    while !shared.stop.load(Ordering::Relaxed) {
        let now = shared.now_us();
        members.apply(now, &mut inbound, &mut events);
        let other_sent = shared.last_sent_us.load(Ordering::Relaxed);
        while let Some(packet) = link.poll(now, unix_ms(), other_sent) {
            let _ = socket.send(&packet);
        }
        // Wake up for the link's next deadline, and at least every 50 ms
        // to notice a stop.
        let wait_us = link
            .next_deadline_us(other_sent)
            .saturating_sub(now)
            .clamp(1_000, 50_000);
        let readable = match socket::wait_readable(socket, Duration::from_micros(wait_us)) {
            Ok(readable) => readable,
            // Not expected on a valid socket; never let it spin a core.
            Err(_) => {
                std::thread::sleep(Duration::from_millis(1));
                false
            }
        };
        // Everything that arrived, then wait again. An error (an ICMP
        // "port unreachable" while the relay restarts, say) ends the batch;
        // the link notices a relay that stays silent.
        while readable && let Ok(n) = socket.recv(&mut buf) {
            let at = shared.now_us();
            let packet = &buf[..n];
            match link.handle(at, packet) {
                Ok(true) => {}
                _ => {
                    if let Some(event) = inbound.handle(packet)
                        && events.push(ToPlayback::Event(at, event)).is_err()
                    {
                        // Playback stalled, or is gone because the call is
                        // ending.
                        queue_full += 1;
                    }
                }
            }
        }
        if now >= next_stats_us {
            next_stats_us = now + 100_000;
            publish_network(shared, &link, &inbound, queue_full);
        }
    }
    // The final numbers, after the last packet.
    publish_network(shared, &link, &inbound, queue_full);
}

fn publish_network(shared: &Shared, link: &Link, inbound: &Inbound, queue_full: u64) {
    let mut stats = shared.stats.lock().expect("not poisoned");
    stats.link = link.stats();
    stats.inbound = inbound.stats();
    stats.playback_queue_full = queue_full;
}

/// What the capture callback owns. Its statistics are published with
/// `try_lock` while it runs, and once more, waiting for the lock, when the
/// device drops the callback, so the final numbers are never stale.
struct Capture {
    socket: Arc<UdpSocket>,
    shared: Arc<Shared>,
    sender: Sender,
    pending: Vec<f32>,
    packets: Packets,
    send_dropped: u64,
    send_errors: u64,
}

impl Capture {
    fn process(&mut self, samples: &[f32]) {
        let frame = self.sender.config().frame.samples();
        let mut rest = samples;
        while !rest.is_empty() {
            let take = (frame - self.pending.len()).min(rest.len());
            self.pending.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.pending.len() < frame {
                break;
            }
            // An Opus error drops this one frame; the next may be fine.
            let _ = self.sender.process(&self.pending, &mut self.packets);
            self.pending.clear();
            for packet in self.packets.iter() {
                match self.socket.send(packet) {
                    Ok(_) => {}
                    // Waiting for room would delay this frame and every
                    // frame after it; a lost packet is concealed instead.
                    Err(e) if is_full(&e) => self.send_dropped += 1,
                    Err(_) => self.send_errors += 1,
                }
            }
            if !self.packets.is_empty() {
                let now = self.shared.now_us();
                self.shared.last_sent_us.store(now, Ordering::Relaxed);
            }
        }
        if let Ok(mut stats) = self.shared.stats.try_lock() {
            self.publish(&mut stats);
        }
    }

    fn publish(&self, stats: &mut CallStats) {
        stats.sender = self.sender.stats();
        stats.talking = self.sender.is_talking();
        stats.send_dropped = self.send_dropped;
        stats.send_errors = self.send_errors;
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Ok(mut stats) = self.shared.stats.lock() {
            self.publish(&mut stats);
            stats.talking = false;
        }
    }
}

/// Whether a send failed because the OS had no room for the packet: a full
/// send buffer (`WouldBlock`), or, on macOS and the BSDs, a full interface
/// queue (`ENOBUFS`), which they report for UDP instead.
fn is_full(e: &std::io::Error) -> bool {
    #[cfg(unix)]
    if e.raw_os_error() == Some(libc::ENOBUFS) {
        return true;
    }
    e.kind() == ErrorKind::WouldBlock
}

fn capture_callback(socket: Arc<UdpSocket>, shared: Arc<Shared>, sender: Sender) -> InputCallback {
    let frame = sender.config().frame.samples();
    let mut capture = Capture {
        socket,
        shared,
        sender,
        pending: Vec::with_capacity(frame),
        packets: Packets::new(),
        send_dropped: 0,
        send_errors: 0,
    };
    Box::new(move |samples: &[f32]| capture.process(samples))
}

/// What the playback callback owns; statistics as for [`Capture`].
struct Playback {
    shared: Arc<Shared>,
    events: Consumer<ToPlayback>,
    mixer: Mixer,
    next_stats_us: u64,
}

impl Playback {
    fn render(&mut self, out: &mut [f32]) {
        while let Ok(message) = self.events.pop() {
            match message {
                // A track whose decoder cannot be made stays silent.
                ToPlayback::Event(at, event) => {
                    let _ = self.mixer.push(at, event);
                }
                // Room was reserved for MAX_TRACKS, and the network thread
                // never makes more: this never allocates.
                ToPlayback::Slot(slot) => self.mixer.add_slot(slot),
            }
        }
        let now = self.shared.now_us();
        self.mixer.render(now, out);
        if now >= self.next_stats_us
            && let Ok(mut stats) = self.shared.stats.try_lock()
        {
            self.next_stats_us = now + 100_000;
            // Copied into the room made in `Call::join`.
            stats.tracks.clear();
            stats.tracks.extend(self.mixer.track_stats());
            stats.departed = self.mixer.departed();
        }
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        if let Ok(mut stats) = self.shared.stats.lock() {
            stats.tracks = self.mixer.stats();
            stats.departed = self.mixer.departed();
        }
    }
}

fn playback_callback(
    shared: Arc<Shared>,
    events: Consumer<ToPlayback>,
    mixer: Mixer,
) -> OutputCallback {
    let mut playback = Playback {
        shared,
        events,
        mixer,
        next_stats_us: 0,
    };
    Box::new(move |out: &mut [f32]| playback.render(out))
}
