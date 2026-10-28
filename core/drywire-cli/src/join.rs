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
use drywire_proto::crypto::VerifyingKey;

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
    let input = match &args.input {
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

    // The event stream before the session, so no arrival is missed.
    let (events, events_thread) = follow(&client, &space.space_id)?;
    let joined = client.join(&room.id).map_err(|e| e.to_string())?;
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
        space_key: space.space_key()?,
        session: me,
        session_secret: joined.session_secret.0,
        identity,
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
        let device = file_device.insert(FileIo::new(source, FILE_PERIOD, duration));
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

    let started = Instant::now();
    let mut next_line = Duration::from_secs(1);
    let mut ended = None;
    let mut snapshots = 0;
    while ended.is_none() {
        std::thread::sleep(Duration::from_millis(50));
        while let Ok(event) = events.try_recv() {
            // The first snapshot was taken before this client joined; the
            // join answer is newer. Later ones follow a reconnection.
            if matches!(event, Event::Snapshot(_)) {
                snapshots += 1;
                if snapshots == 1 {
                    continue;
                }
            }
            ended = ended.or(on_event(
                event,
                &room,
                me,
                &mut names,
                &call_members,
                args.quiet,
            ));
        }
        let finished = match &file_device {
            Some(device) => device.is_finished(),
            None => duration.is_some_and(|d| started.elapsed() >= d),
        };
        if finished {
            break;
        }
        if !args.quiet && started.elapsed() >= next_line {
            eprintln!(
                "[{:>4}s] {}",
                next_line.as_secs(),
                status_line(&names, &call.stats())
            );
            next_line += Duration::from_secs(1);
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
    // event thread stops when it next hears from the server.
    let _ = client.leave();
    drop(events);
    drop(events_thread);
    let stats = call.stats();
    eprintln!("{}", status_line(&names, &stats));
    println!("{}", summary(&stats));

    if let (Some(path), Some(samples)) = (&args.output, recorded) {
        wav::write(path, &samples).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    match ended {
        Some(why) => Err(why),
        None => Ok(()),
    }
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

/// Reads the space's events on a thread of its own, opening the stream
/// again whenever it ends, until the receiver is dropped.
fn follow(
    client: &Arc<Client>,
    space: &drywire_control::Id,
) -> Result<(Receiver<Event>, std::thread::JoinHandle<()>), String> {
    let first = client.events(space).map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::channel();
    let (client, space) = (Arc::clone(client), *space);
    let thread = std::thread::Builder::new()
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
        .map_err(|e| e.to_string())?;
    Ok((rx, thread))
}

/// Applies one event to the call. Returns why the call must end, if it
/// must: this client's own session ended.
fn on_event(
    event: Event,
    room: &Room,
    me: u32,
    names: &mut HashMap<u32, String>,
    call: &CallMembers,
    quiet: bool,
) -> Option<String> {
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
                return Some("this session ended while the event stream was down".into());
            }
            names.retain(|session, _| {
                let keep = *session == me || present.contains(session);
                if !keep {
                    call.remove(*session);
                }
                keep
            });
        }
        Event::Leave(p) | Event::Disconnect(p) if p.session == me => {
            return Some(
                "the server ended this session (joined elsewhere, removed, or silent too                  long): join again"
                    .into(),
            );
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
fn status_line(names: &HashMap<u32, String>, stats: &CallStats) -> String {
    let link = &stats.link;
    let mut line = match link.rtt_us {
        Some(rtt) => format!(
            "rtt {} (min {})",
            ms(rtt),
            ms(link.min_rtt_us.unwrap_or(rtt))
        ),
        None if link.bound => "rtt -".into(),
        None => "not bound".into(),
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
         received_audio={} no_key={} bad_tag={} bad_signature={} duplicate={} \
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
