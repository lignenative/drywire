//! `drywire join`: one client in a call, on the sound card or on files.
//!
//! It asks the control plane for a session in a room, starts the call on
//! the relay it names, and follows the space's events: members who join
//! or leave the room are added to or removed from the running call.

use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use drywire_control::{Client, Event, Presence, Room};
use drywire_proto::crypto::{SigningKey, VerifyingKey};

use drywire_core::audio::file::{FileIo, Source};
use drywire_core::audio::{AudioIo, AudioOptions, SAMPLE_RATE, default_devices};
use drywire_core::call::{Call, CallMembers, CallOptions, CallStats};
use drywire_core::config::{ClientConfig, Member};
use drywire_core::jitter::JitterStats;
use drywire_core::link::LinkConfig;
use drywire_core::sender::SenderConfig;
use drywire_core::wav;

use crate::account::accept_link;
use crate::args::Join;
use crate::home::{Home, printable};

/// The period of the file device: 2.5 ms, the smallest frame, so that a
/// file-mode client adds as little latency as a fast sound card.
const FILE_PERIOD: usize = 120;

/// How often the event stream is opened again after it ends.
const EVENTS_RETRY: Duration = Duration::from_secs(2);

/// Waits before joining again after the server forgot this session (ADR
/// 0035, as the apps do): 1, 2, 4, 8 seconds, then every 15.
const REJOIN_FIRST: Duration = Duration::from_secs(1);
const REJOIN_MAX: Duration = Duration::from_secs(15);

/// A relay that answers no ping for this long has forgotten the session
/// (it restarted, or the server ended it unseen): join again. The server
/// itself ends a session it has not heard from for 30 seconds.
const RELAY_SILENCE: Duration = Duration::from_secs(20);

/// A round trip older than this is not shown: the relay stopped answering.
const PONG_SILENCE: Duration = Duration::from_secs(3);

/// Why the server ended this client's session.
enum End {
    /// It forgot it (the relay heard nothing, or it restarted): join again.
    Lost(String),
    /// Another device of this account joined, or the member was removed.
    Final(String),
}

/// How one call ended.
enum Outcome {
    /// Stop here: asked to, finished, or refused for good.
    Done(Result<(), String>),
    /// The server forgot this session, or could not be reached to join:
    /// a live client joins again.
    Lost(String),
}

pub fn run(args: Join) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    let identity = home.identity()?;
    let space = match &args.link {
        Some(link) => accept_link(&home, link, &args.display_name)?,
        None => home.find(args.space.as_deref())?,
    };
    let client =
        Arc::new(Client::new(&space.instance, identity.clone()).map_err(|e| e.to_string())?);
    let view = client.space(&space.space_id).map_err(|e| e.to_string())?;
    let room = pick_room(&view.rooms, args.room.as_deref())?;

    // Read the input before joining, so a bad file fails early.
    let mut input = match &args.input {
        Some(path) => Some(wav::read(path).map_err(|e| e.to_string())?),
        None => None,
    };
    let duration = args.duration.or_else(|| {
        input.as_ref().map(|s| {
            Duration::from_secs_f64(s.len() as f64 / f64::from(SAMPLE_RATE))
                + Duration::from_secs(1)
        })
    });
    if args.file_mode() && duration.is_none() && !args.echo {
        return Err("with --output alone, say how long to record with --duration".into());
    }
    // A call on the sound card, or an echo bot, joins again when the
    // server forgets it; a call on files is a measurement, which ends.
    let rejoins = args.input.is_none() && args.output.is_none();

    // The event stream before the session, so no arrival is missed.
    let (events, events_thread) = follow(&client, &space.space_id)?;
    crate::stop::install();
    let started = Instant::now();
    let mut wait = REJOIN_FIRST;
    let mut first = true;
    let result = loop {
        let outcome = one_call(
            &args,
            &identity,
            &client,
            &space,
            &room,
            &events,
            input.take(),
            duration,
            started,
            first,
        );
        first = false;
        match outcome {
            Outcome::Done(result) => break result,
            Outcome::Lost(why) if rejoins => {
                eprintln!("{why}: joining again in {} s", wait.as_secs());
                if sleep_unless_stopped(wait) {
                    break Ok(());
                }
                wait = (wait * 2).min(REJOIN_MAX);
            }
            Outcome::Lost(why) => break Err(why),
        }
    };
    drop(events);
    drop(events_thread);
    result
}

/// Sleeps, or returns true at once when Ctrl-C asks to stop.
fn sleep_unless_stopped(d: Duration) -> bool {
    let until = Instant::now() + d;
    while Instant::now() < until {
        if crate::stop::requested() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// One session: join, run the call until it ends, leave.
#[allow(clippy::too_many_arguments)]
fn one_call(
    args: &Join,
    identity: &SigningKey,
    client: &Arc<Client>,
    space: &crate::home::KnownSpace,
    room: &Room,
    events: &Receiver<Stamped>,
    input: Option<Vec<f32>>,
    duration: Option<Duration>,
    started: Instant,
    first: bool,
) -> Outcome {
    let joined = match client.join(&room.id) {
        Ok(joined) => joined,
        // The first join says what is wrong; a later one waits for the
        // network or the server to come back, unless it was refused.
        Err(e) if !first && !matches!(e.status(), Some(403 | 404)) => {
            return Outcome::Lost(format!("could not join again ({e})"));
        }
        Err(e) => return Outcome::Done(Err(e.to_string())),
    };
    match call_on(
        args, identity, client, space, room, events, input, duration, started, joined,
    ) {
        Ok(outcome) => outcome,
        Err(e) => Outcome::Done(Err(e)),
    }
}

#[allow(clippy::too_many_arguments)]
fn call_on(
    args: &Join,
    identity: &SigningKey,
    client: &Arc<Client>,
    space: &crate::home::KnownSpace,
    room: &Room,
    events: &Receiver<Stamped>,
    input: Option<Vec<f32>>,
    duration: Option<Duration>,
    started: Instant,
    joined: drywire_control::JoinAnswer,
) -> Result<Outcome, String> {
    let joined_at = Instant::now();
    let relay = resolve(&joined.relay)?;
    let me = joined.session;
    let mut names: HashMap<u32, String> = HashMap::new();
    let mut members = Vec::new();
    for p in joined.sessions.iter().filter(|p| p.room_id == room.id) {
        names.insert(p.session, printable(&p.name));
        members.push(Member {
            session: p.session,
            name: printable(&p.name),
            public_key: public_key(p)?,
        });
    }
    let config = ClientConfig {
        room_id: room.id.0,
        // Until the space has an MLS group (ADR 0052, step 4), the room
        // key comes from the space key.
        room_key: drywire_proto::crypto::room_key(&space.space_key()?, &room.id.0),
        session: me,
        session_secret: joined.session_secret.0,
        identity: identity.clone(),
        members,
    };

    let options = CallOptions {
        relay,
        sender: SenderConfig {
            bitrate: args.bitrate,
            gate: args.gate,
            redundancy: args.redundancy,
            ..SenderConfig::new(args.frame)
        },
        link: LinkConfig {
            ping_interval_us: args.ping.then_some(1_000_000),
        },
        join_timeout: Duration::from_secs(5),
        min_buffer_us: (args.min_buffer_ms * 1000.0) as u64,
    };
    let (mut call, capture, playback) = Call::join(&config, options).map_err(|e| e.to_string())?;
    let call_members = call.members();
    eprintln!(
        "joined as {} (session {me}) in {} / {} via {relay}: {} frames, {} kb/s{}",
        names.get(&me).map_or("?", String::as_str),
        space.name,
        printable(&room.name),
        args.frame,
        args.bitrate / 1000,
        if args.redundancy { ", redundancy" } else { "" }
    );

    let mut file_device = None;
    let mut hardware = None;
    let streams = if args.file_mode() {
        let source = match input {
            Some(samples) => Source::Samples(samples),
            None if args.echo => Source::Echo,
            None => Source::Samples(Vec::new()),
        };
        let left = duration.map(|d| d.saturating_sub(started.elapsed()));
        let device = file_device.insert(FileIo::new(source, FILE_PERIOD, left));
        device.start(capture, playback)
    } else {
        let options = AudioOptions {
            buffer_frames: args.buffer,
        };
        let device = hardware.insert(default_devices(options).map_err(|e| e.to_string())?);
        device.start(capture, playback)
    }
    .map_err(|e| e.to_string())?;
    for stream in &streams {
        eprintln!("{stream}");
    }

    let mut next_line = started.elapsed().as_secs() + 1;
    let mut ended = None;
    // Events still queued from the last session speak of its session
    // number, not this one's; a snapshot from a stream opened before this
    // join is let through by `fresh` below.
    let mut pongs = 0;
    let mut last_pong = Instant::now();
    while ended.is_none() {
        std::thread::sleep(Duration::from_millis(50));
        while let Ok((opened, event)) = events.try_recv() {
            // A stream opened before this join gave a snapshot without it.
            let fresh = opened < joined_at;
            ended = ended.or(on_event(
                event,
                room,
                me,
                &mut names,
                &call_members,
                args.quiet,
                fresh,
            ));
        }
        if crate::stop::requested() {
            eprintln!("leaving");
            break;
        }
        let finished = match &file_device {
            Some(device) => device.is_finished(),
            None => duration.is_some_and(|d| started.elapsed() >= d),
        };
        if finished {
            break;
        }
        let stats = call.stats();
        if stats.link.pongs != pongs {
            pongs = stats.link.pongs;
            last_pong = Instant::now();
        }
        // A restarted server does not know this session, and says nothing
        // of it: its relay's silence is the only sign.
        if args.ping && last_pong.elapsed() > RELAY_SILENCE {
            ended = Some(End::Lost(format!(
                "the relay has not answered for {} s",
                last_pong.elapsed().as_secs()
            )));
        }
        if !args.quiet && started.elapsed().as_secs() >= next_line {
            let silent =
                (args.ping && last_pong.elapsed() > PONG_SILENCE).then(|| last_pong.elapsed());
            eprintln!("[{next_line:>4}s] {}", status_line(&names, &stats, silent));
            next_line += 1;
        }
    }

    // Stop the audio first, then the network, then read the final numbers.
    if let Some(device) = &mut hardware {
        device.stop();
    }
    let recorded = file_device.as_mut().map(|device| {
        device.stop();
        device.recorded()
    });
    call.leave();
    // Free the room at once rather than after the relay's timeout. The
    // server ends it anyway if it already forgot this session.
    let _ = client.leave();
    let stats = call.stats();
    eprintln!("{}", status_line(&names, &stats, None));
    println!("{}", summary(&stats));

    if let (Some(path), Some(samples)) = (&args.output, recorded) {
        wav::write(path, &samples).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(match ended {
        Some(End::Lost(why)) => Outcome::Lost(why),
        Some(End::Final(why)) => Outcome::Done(Err(why)),
        None => Outcome::Done(Ok(())),
    })
}

/// The room named `wanted`, or the first one by name.
fn pick_room(rooms: &[Room], wanted: Option<&str>) -> Result<Room, String> {
    match wanted {
        Some(name) => rooms
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(name))
            .cloned()
            .ok_or_else(|| {
                let names: Vec<String> = rooms.iter().map(|r| printable(&r.name)).collect();
                format!("no room {name:?}; the rooms are: {}", names.join(", "))
            }),
        None => rooms
            .first()
            .cloned()
            .ok_or_else(|| "this space has no room".to_string()),
    }
}

fn resolve(relay: &str) -> Result<SocketAddr, String> {
    relay
        .to_socket_addrs()
        .map_err(|e| format!("relay {relay}: {e}"))?
        .next()
        .ok_or_else(|| format!("relay {relay}: no address"))
}

fn public_key(p: &Presence) -> Result<VerifyingKey, String> {
    VerifyingKey::from_bytes(&p.public_key.0)
        .map_err(|_| format!("the server sent an invalid key for session {}", p.session))
}

/// An event, with when its stream was opened: a snapshot speaks of that
/// moment.
type Stamped = (Instant, Event);

/// Reads the space's events on a thread of its own, opening the stream
/// again whenever it ends, until the receiver is dropped.
fn follow(
    client: &Arc<Client>,
    space: &drywire_control::Id,
) -> Result<(Receiver<Stamped>, std::thread::JoinHandle<()>), String> {
    let first = (
        Instant::now(),
        client.events(space).map_err(|e| e.to_string())?,
    );
    let (tx, rx) = mpsc::channel();
    let (client, space) = (Arc::clone(client), *space);
    let thread = std::thread::Builder::new()
        .name("drywire-events".into())
        .spawn(move || {
            let mut stream = Some(first);
            loop {
                if let Some((opened, events)) = stream.take() {
                    for event in events.flatten() {
                        if tx.send((opened, event)).is_err() {
                            return;
                        }
                    }
                }
                std::thread::sleep(EVENTS_RETRY);
                let opened = Instant::now();
                match client.events(&space) {
                    Ok(events) => stream = Some((opened, events)),
                    // No longer a member, or the space is gone: stop.
                    Err(e) if e.status() == Some(404) => return,
                    Err(_) => {}
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok((rx, thread))
}

/// Applies one event to the call. Returns why the call must end, if it
/// must: this client's own session ended. `fresh`: the call only began,
/// and a snapshot without this client may be older than it.
fn on_event(
    event: Event,
    room: &Room,
    me: u32,
    names: &mut HashMap<u32, String>,
    call: &CallMembers,
    quiet: bool,
    fresh: bool,
) -> Option<End> {
    let add = |p: &Presence, names: &mut HashMap<u32, String>| {
        if p.room_id != room.id || p.session == me || names.contains_key(&p.session) {
            return;
        }
        if let Ok(key) = public_key(p) {
            call.add(p.session, key);
            names.insert(p.session, printable(&p.name));
            if !quiet {
                eprintln!("{} joined", printable(&p.name));
            }
        }
    };
    match event {
        Event::Join(p) => add(&p, names),
        Event::Snapshot(everyone) => {
            // After the stream was opened again: whoever came meanwhile
            // is added, whoever left is removed.
            for p in &everyone {
                add(p, names);
            }
            let present: Vec<u32> = everyone
                .iter()
                .filter(|p| p.room_id == room.id)
                .map(|p| p.session)
                .collect();
            if !present.contains(&me) {
                if fresh {
                    return None;
                }
                return Some(End::Lost(
                    "this session ended while the event stream was down".into(),
                ));
            }
            names.retain(|session, _| {
                let keep = *session == me || present.contains(session);
                if !keep {
                    call.remove(*session);
                }
                keep
            });
        }
        Event::Disconnect(p) if p.session == me => {
            return Some(End::Lost(
                "the relay stopped hearing this client, and the server ended the session".into(),
            ));
        }
        Event::Leave(p) if p.session == me => {
            return Some(End::Final(
                "the server ended this session: this account joined from another device, or \
                 was removed"
                    .into(),
            ));
        }
        Event::Leave(p) | Event::Disconnect(p) => {
            if let Some(name) = names.remove(&p.session) {
                call.remove(p.session);
                if !quiet {
                    eprintln!("{name} left");
                }
            }
        }
        Event::Other(_) => {}
    }
    None
}

fn ms(us: u64) -> String {
    format!("{:.2} ms", us as f64 / 1000.0)
}

/// One human-readable line about the call.
/// One line of numbers. `silent`: how long the relay has not answered a
/// ping, when that is long enough that the last round trip means nothing.
fn status_line(
    names: &HashMap<u32, String>,
    stats: &CallStats,
    silent: Option<Duration>,
) -> String {
    let link = &stats.link;
    let mut line = match (silent, link.rtt_us) {
        (Some(s), _) => format!("rtt - (no answer for {} s)", s.as_secs()),
        (None, Some(rtt)) => format!(
            "rtt {} (min {})",
            ms(rtt),
            ms(link.min_rtt_us.unwrap_or(rtt))
        ),
        (None, None) if link.bound => "rtt -".into(),
        (None, None) => "not bound".into(),
    };
    line += &format!(
        " | up {} sent, {} lost{}",
        stats.sender.audio,
        link.uplink_lost,
        if stats.talking { ", talking" } else { "" }
    );
    if stats.send_dropped + stats.send_errors > 0 {
        line += &format!(
            ", not sent: {} socket full, {} errors",
            stats.send_dropped, stats.send_errors
        );
    }
    for track in &stats.tracks {
        let name = names
            .get(&track.session)
            .cloned()
            .unwrap_or_else(|| track.session.to_string());
        let j = &track.jitter;
        line += &format!(
            " | {name}: buffer {} (target {}), played {}, concealed {}, late {}, dropped {}",
            track.depth,
            ms(j.target_us),
            j.played,
            j.concealed,
            j.late,
            j.dropped
        );
    }
    let i = &stats.inbound;
    let errors = i.no_key + i.bad_tag + i.bad_signature + i.bad_framing + i.invalid;
    if errors > 0 {
        line += &format!(
            " | dropped: no_key {}, bad_tag {}, bad_signature {}, other {}",
            i.no_key,
            i.bad_tag,
            i.bad_signature,
            i.bad_framing + i.invalid
        );
    }
    // Expected once per newcomer: shown apart, never among the errors.
    if i.unknown_sender > 0 {
        line += &format!(" | early announcements {}", i.unknown_sender);
    }
    line
}

/// The final counters as one line of `key=value` pairs, for scripts and
/// tests. Track counters are summed over every sender.
fn summary(stats: &CallStats) -> String {
    // Every sender, those who left included.
    let sum = |f: fn(&JitterStats) -> u64| -> u64 {
        stats.tracks.iter().map(|t| f(&t.jitter)).sum::<u64>() + f(&stats.departed)
    };
    let i = &stats.inbound;
    format!(
        "summary sent_audio={} announces={} spurts={} send_dropped={} send_errors={} \
         received_audio={} no_key={} bad_tag={} bad_signature={} unknown_sender={} duplicate={} \
         played={} concealed={} late={} dropped={} playback_queue_full={} pongs={} \
         uplink_received={} uplink_lost={}",
        stats.sender.audio,
        stats.sender.announces,
        stats.sender.spurts,
        stats.send_dropped,
        stats.send_errors,
        i.audio,
        i.no_key,
        i.bad_tag,
        i.bad_signature,
        i.unknown_sender,
        i.duplicate,
        sum(|j| j.played),
        sum(|j| j.concealed),
        sum(|j| j.late),
        sum(|j| j.dropped),
        stats.playback_queue_full,
        stats.link.pongs,
        stats.link.uplink_received,
        stats.link.uplink_lost,
    )
}
