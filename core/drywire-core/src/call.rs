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
//!   passes decrypted frames to playback through a channel.
//! - **Playback** (the audio output callback): drains that channel into the
//!   [`Mixer`] and renders the mix.
//!
//! The audio callbacks never wait for a lock: they update the shared
//! statistics only when the lock is free.

use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender as ChannelSender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use drywire_proto::header::MAX_PACKET_SIZE;

use crate::audio::{InputCallback, OutputCallback};
use crate::config::ClientConfig;
use crate::inbound::{Event, Inbound, InboundStats};
use crate::link::{Link, LinkConfig, LinkStats};
use crate::mixer::{Mixer, TrackStats};
use crate::opus::OpusError;
use crate::sender::{Sender, SenderConfig, SenderStats};
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
    /// Each sender's playback.
    pub tracks: Vec<TrackStats>,
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
                "the relay did not answer: check its address, that it runs, and that this \
                 configuration's session and secret are in its rooms file",
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

        let shared = Arc::new(Shared {
            started: Instant::now(),
            stop: AtomicBool::new(false),
            last_sent_us: AtomicU64::new(0),
            stats: Mutex::new(CallStats::default()),
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

        let others = config
            .members
            .iter()
            .filter(|m| m.session != config.session)
            .map(|m| (m.session, m.public_key));
        let inbound = Inbound::new(config.room_id, &config.space_key, others);
        let (events, playback_events) = channel();

        let network = {
            let socket = Arc::clone(&socket);
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("drywire-network".into())
                .spawn(move || network_loop(&socket, &shared, link, inbound, events))?
        };
        let input = capture_callback(socket, Arc::clone(&shared), sender);
        let output = playback_callback(
            Arc::clone(&shared),
            playback_events,
            Mixer::with_min_target(options.min_buffer_us),
        );
        Ok((
            Call {
                shared,
                network: Some(network),
            },
            input,
            output,
        ))
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

/// Receives, decrypts and forwards to playback; drives the link.
fn network_loop(
    socket: &UdpSocket,
    shared: &Shared,
    mut link: Link,
    mut inbound: Inbound,
    events: ChannelSender<(u64, Event)>,
) {
    let mut buf = [0u8; MAX_PACKET_SIZE];
    let mut next_stats_us = 0;
    while !shared.stop.load(Ordering::Relaxed) {
        let now = shared.now_us();
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
        let readable =
            socket::wait_readable(socket, Duration::from_micros(wait_us)).unwrap_or(false);
        // Everything that arrived, then wait again. An error (an ICMP
        // "port unreachable" while the relay restarts, say) ends the batch;
        // the link notices a relay that stays silent.
        while readable && let Ok(n) = socket.recv(&mut buf) {
            let at = shared.now_us();
            let packet = &buf[..n];
            match link.handle(at, packet) {
                Ok(true) => {}
                _ => {
                    if let Some(event) = inbound.handle(packet) {
                        // Playback gone means the call is ending.
                        let _ = events.send((at, event));
                    }
                }
            }
        }
        if now >= next_stats_us {
            next_stats_us = now + 100_000;
            publish_network(shared, &link, &inbound);
        }
    }
    // The final numbers, after the last packet.
    publish_network(shared, &link, &inbound);
}

fn publish_network(shared: &Shared, link: &Link, inbound: &Inbound) {
    let mut stats = shared.stats.lock().expect("not poisoned");
    stats.link = link.stats();
    stats.inbound = inbound.stats();
}

/// What the capture callback owns. Its statistics are published with
/// `try_lock` while it runs, and once more, waiting for the lock, when the
/// device drops the callback, so the final numbers are never stale.
struct Capture {
    socket: Arc<UdpSocket>,
    shared: Arc<Shared>,
    sender: Sender,
    pending: Vec<f32>,
    packets: Vec<Vec<u8>>,
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
            self.packets.clear();
            // An Opus error drops this one frame; the next may be fine.
            let _ = self.sender.process(&self.pending, &mut self.packets);
            self.pending.clear();
            for packet in &self.packets {
                match self.socket.send(packet) {
                    Ok(_) => {}
                    // Waiting for room would delay this frame and every
                    // frame after it; a lost packet is concealed instead.
                    Err(e) if e.kind() == ErrorKind::WouldBlock => self.send_dropped += 1,
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

fn capture_callback(socket: Arc<UdpSocket>, shared: Arc<Shared>, sender: Sender) -> InputCallback {
    let frame = sender.config().frame.samples();
    let mut capture = Capture {
        socket,
        shared,
        sender,
        pending: Vec::with_capacity(frame),
        packets: Vec::with_capacity(4),
        send_dropped: 0,
        send_errors: 0,
    };
    Box::new(move |samples: &[f32]| capture.process(samples))
}

/// What the playback callback owns; statistics as for [`Capture`].
struct Playback {
    shared: Arc<Shared>,
    events: Receiver<(u64, Event)>,
    mixer: Mixer,
    next_stats_us: u64,
}

impl Playback {
    fn render(&mut self, out: &mut [f32]) {
        while let Ok((at, event)) = self.events.try_recv() {
            // A track whose decoder cannot be made stays silent.
            let _ = self.mixer.push(at, event);
        }
        let now = self.shared.now_us();
        self.mixer.render(now, out);
        if now >= self.next_stats_us
            && let Ok(mut stats) = self.shared.stats.try_lock()
        {
            self.next_stats_us = now + 100_000;
            stats.tracks = self.mixer.stats();
        }
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        if let Ok(mut stats) = self.shared.stats.lock() {
            stats.tracks = self.mixer.stats();
        }
    }
}

fn playback_callback(
    shared: Arc<Shared>,
    events: Receiver<(u64, Event)>,
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
