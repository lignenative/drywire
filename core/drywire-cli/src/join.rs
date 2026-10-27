//! `drywire-cli join`: one client in a call, on the sound card or on files.

use std::time::{Duration, Instant};

use drywire_core::audio::file::{FileIo, Source};
use drywire_core::audio::{AudioIo, AudioOptions, SAMPLE_RATE, default_devices};
use drywire_core::call::{Call, CallOptions, CallStats};
use drywire_core::config::ClientConfig;
use drywire_core::link::LinkConfig;
use drywire_core::sender::SenderConfig;
use drywire_core::wav;

use crate::args::Join;

/// The period of the file device: 2.5 ms, the smallest frame, so that a
/// file-mode client adds as little latency as a fast sound card.
const FILE_PERIOD: usize = 120;

pub fn run(args: Join) -> Result<(), String> {
    let config = ClientConfig::load(&args.config).map_err(|e| e.to_string())?;
    let relay = config
        .check_relay(args.relay.as_deref())
        .map_err(|e| e.to_string())?;

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
    let me = config.me();
    eprintln!(
        "joined as {} (session {}) via {relay}: {} frames, {} kb/s{}",
        me.name,
        me.session,
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
    loop {
        std::thread::sleep(Duration::from_millis(50));
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
                status_line(&config, &call.stats())
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
    let stats = call.stats();
    eprintln!("{}", status_line(&config, &stats));
    println!("{}", summary(&stats));

    if let (Some(path), Some(samples)) = (&args.output, recorded) {
        wav::write(path, &samples).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

fn ms(us: u64) -> String {
    format!("{:.2} ms", us as f64 / 1000.0)
}

/// One human-readable line about the call.
fn status_line(config: &ClientConfig, stats: &CallStats) -> String {
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
    for track in &stats.tracks {
        let name = config
            .member(track.session)
            .map_or_else(|| track.session.to_string(), |m| m.name.clone());
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
    let sum = |f: fn(&drywire_core::mixer::TrackStats) -> u64| -> u64 {
        stats.tracks.iter().map(f).sum()
    };
    let i = &stats.inbound;
    format!(
        "summary sent_audio={} announces={} spurts={} send_errors={} \
         received_audio={} no_key={} bad_tag={} bad_signature={} duplicate={} \
         played={} concealed={} late={} dropped={} pongs={} uplink_received={} uplink_lost={}",
        stats.sender.audio,
        stats.sender.announces,
        stats.sender.spurts,
        stats.send_errors,
        i.audio,
        i.no_key,
        i.bad_tag,
        i.bad_signature,
        i.duplicate,
        sum(|t| t.jitter.played),
        sum(|t| t.jitter.concealed),
        sum(|t| t.jitter.late),
        sum(|t| t.jitter.dropped),
        stats.link.pongs,
        stats.link.uplink_received,
        stats.link.uplink_lost,
    )
}
