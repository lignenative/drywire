//! Audio from one client to another through the real Go relay:
//! gate → Opus → seal → UDP → relay → UDP → open → jitter buffer → decode.
//!
//! Two clients of the development room "lobby" bind their sessions with
//! `hello`. The first one announces its key, then talks for 600 ms in real
//! time, at each frame size. The second records what it receives, over the
//! real network path, then plays it through the mixer on a simulated clock:
//! each packet arrives exactly one capture period after the previous one, as
//! its timestamp says. The network part (relay, sockets, encryption) is
//! real and checked exactly; the playback part does not depend on how the
//! test machine schedules threads, so it is checked exactly too. On the
//! way, the first client pings the relay and sends a keepalive, and the
//! relay's own counters must show nothing dropped.

#[path = "../../testsupport/relay.rs"]
mod relay;

use std::net::UdpSocket;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use drywire_core::inbound::Inbound;
use drywire_core::link::{Link, LinkConfig};
use drywire_core::mixer::Mixer;
use drywire_core::opus::{FrameSize, SAMPLE_RATE};
use drywire_core::sender::{Packets, Sender, SenderConfig};
use drywire_proto::control::{new_keepalive, new_ping};
use drywire_proto::crypto::SigningKey;
use drywire_proto::{PacketType, parse};

use relay::{Relay, dev_secret};

const ROOM_ID: [u8; 16] = [0x0f; 16];
const SPACE_KEY: [u8; 32] = [0x5a; 32];
/// Opus delays the signal by 2.5 ms (`opus::tests` pins it).
const LOOKAHEAD: usize = 120;

struct Client {
    socket: UdpSocket,
    link: Link,
    started: Instant,
}

impl Client {
    /// Binds `session` to a fresh socket: hello until the relay answers.
    fn join(relay: &Relay, session: u32) -> Client {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.connect(relay.udp).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let mut client = Client {
            socket,
            link: Link::new(session, dev_secret(session), LinkConfig::default()),
            started: Instant::now(),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !client.link.is_bound() {
            assert!(Instant::now() < deadline, "the relay never acknowledged");
            if let Some(hello) = client.link.poll(client.now_us(), unix_ms(), 0) {
                assert_eq!(parse(&hello).unwrap().packet_type, PacketType::Hello);
                client.socket.send(&hello).unwrap();
            }
            client.receive_control();
        }
        client
    }

    fn now_us(&self) -> u64 {
        self.started.elapsed().as_micros() as u64
    }

    /// Waits a little for a hello_ack or pong and hands it to the link.
    fn receive_control(&mut self) {
        let mut buf = [0u8; 1500];
        if let Ok(n) = self.socket.recv(&mut buf) {
            let now = self.now_us();
            self.link.handle(now, &buf[..n]).unwrap();
        }
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// 200 ms of silence, 600 ms of two mixed tones, 400 ms of silence: the
/// gate opens on the tones and closes during the last silence.
fn speech() -> Vec<f32> {
    let n = |ms: usize| ms * SAMPLE_RATE as usize / 1000;
    let mut signal = vec![0.0; n(200)];
    signal.extend((0..n(600)).map(|i| {
        let t = i as f32 / SAMPLE_RATE as f32;
        let tau = 2.0 * std::f32::consts::PI;
        0.3 * (tau * 440.0 * t).sin() + 0.2 * (tau * 1234.0 * t).sin()
    }));
    signal.extend(vec![0.0; n(400)]);
    signal
}

/// What [`talk`] did, for the simulated playback clock.
struct Talk {
    /// Audio packets sent.
    sent: u64,
    /// When the first input sample was captured.
    started: Instant,
    /// The index of the first frame the gate let through.
    first_audio_frame: u64,
}

/// Sends `input` frame by frame at the pace of a real microphone.
fn talk(alex: &Client, sender: &mut Sender, input: &[f32]) -> Talk {
    let frame = sender.config().frame;
    let mut packets = Packets::new();
    alex.socket.send(&sender.announce()).unwrap();
    let started = Instant::now();
    let mut first_audio_frame = None;
    for (k, pcm) in input.chunks_exact(frame.samples()).enumerate() {
        // A frame is ready once it has been fully captured.
        let ready = started + Duration::from_micros((k as u64 + 1) * frame.micros());
        std::thread::sleep(ready.saturating_duration_since(Instant::now()));
        sender.process(pcm, &mut packets).unwrap();
        if sender.stats().audio > 0 {
            first_audio_frame.get_or_insert(k as u64);
        }
        for p in packets.iter() {
            alex.socket.send(p).unwrap();
        }
    }
    Talk {
        sent: sender.stats().audio,
        started,
        first_audio_frame: first_audio_frame.expect("the gate opened"),
    }
}

/// Records every packet the client receives until `quiet` passes without
/// one, with its arrival time.
fn record(sam: &Client, quiet: Duration) -> Vec<(u64, Vec<u8>)> {
    sam.socket.set_read_timeout(Some(quiet)).unwrap();
    let mut got = Vec::new();
    let mut buf = [0u8; 1500];
    while let Ok(n) = sam.socket.recv(&mut buf) {
        got.push((sam.now_us(), buf[..n].to_vec()));
    }
    got
}

/// Plays the received packets through the mixer, on a simulated clock whose
/// time 0 is the capture of the first input sample, and a sound card that
/// asks for 1 ms (48 samples) at a time.
///
/// Each audio packet is handed to the mixer at the time a perfect network
/// would deliver it: the instant its frame was fully captured, as its
/// timestamp says. Real arrival times depend on the test machine: a sender
/// thread that stalls on a loaded CI runner sends a few frames at once, and
/// the jitter buffer then rightly drops or conceals one, which says nothing
/// about the path under test. With ideal arrivals the whole playback is
/// deterministic, down to the delay of the output, which the test checks
/// exactly. The real timing is still measured and returned, as the largest
/// lateness of a packet against that ideal schedule, for the log.
fn play(
    received: Vec<(u64, Vec<u8>)>,
    recorder: &Client,
    talk: &Talk,
    frame: FrameSize,
    inbound: &mut Inbound,
    samples: usize,
) -> (Vec<f32>, Mixer, u64) {
    let mut mixer = Mixer::new();
    // Time 0 on the recorder's clock, which started first.
    let origin_us = talk.started.duration_since(recorder.started).as_micros() as u64;
    let first_ready_us = (talk.first_audio_frame + 1) * frame.micros();
    // An announcement goes with the packet before it, so the order stays;
    // the ones before the first audio packet are known at time 0.
    let mut first_timestamp = None;
    let mut ideal = 0;
    let mut worst_late_us = 0;
    let scheduled: Vec<(u64, Vec<u8>)> = received
        .into_iter()
        .map(|(real, packet)| {
            let header = parse(&packet).expect("a valid packet");
            if header.packet_type == PacketType::Audio {
                let first = *first_timestamp.get_or_insert(header.timestamp);
                let elapsed = u64::from(header.timestamp.wrapping_sub(first));
                ideal = first_ready_us + elapsed * 1_000_000 / u64::from(SAMPLE_RATE);
                let real = real.saturating_sub(origin_us);
                worst_late_us = worst_late_us.max(real.saturating_sub(ideal));
            }
            (ideal, packet)
        })
        .collect();

    let mut incoming = scheduled.into_iter().peekable();
    let mut output = vec![0.0; samples];
    for (i, chunk) in output.chunks_mut(48).enumerate() {
        let now = i as u64 * 1_000;
        while let Some((at, packet)) = incoming.next_if(|(at, _)| *at <= now) {
            if let Some(event) = inbound.handle(&packet) {
                mixer.push(at, event).unwrap();
            }
        }
        mixer.render(now, chunk);
    }
    (output, mixer, worst_late_us)
}

/// The delay at which `output` best matches `input`, searched up to
/// `max_lag` samples.
fn best_lag(input: &[f32], output: &[f32], max_lag: usize) -> usize {
    let correlation = |lag: usize| -> f32 {
        input
            .iter()
            .zip(output.get(lag..).unwrap_or(&[]))
            .map(|(a, b)| a * b)
            .sum()
    };
    (0..max_lag)
        .max_by(|a, b| correlation(*a).total_cmp(&correlation(*b)))
        .unwrap()
}

fn snr_db(input: &[f32], output: &[f32]) -> f32 {
    let (signal, noise) = input
        .iter()
        .zip(output)
        .fold((0.0f32, 0.0f32), |(s, e), (a, b)| {
            (s + a * a, e + (a - b) * (a - b))
        });
    10.0 * (signal / noise).log10()
}

#[test]
fn audio_crosses_the_relay_at_every_frame_size() {
    let relay = Relay::start();
    let sam = Client::join(&relay, 2);
    let alex_identity = SigningKey::from_bytes(&[0xa1; 32]);
    let input = speech();
    let mut alex = None;
    let mut last_sent = 0;

    for (frame, redundancy) in [
        (FrameSize::Ms2_5, true),
        (FrameSize::Ms5, false),
        (FrameSize::Ms10, false),
    ] {
        let config = SenderConfig {
            redundancy,
            ..SenderConfig::new(frame)
        };
        let mut sender = Sender::new(alex_identity.clone(), 1, ROOM_ID, &SPACE_KEY, config)
            .expect("an Opus encoder");
        // A fresh binding for each run, as a restarted client would make:
        // every sender restarts its sequence at 0 in epoch 0, and the relay
        // resets its loss counters and replay window on a new hello
        // (protocol 10.4). Reusing the binding would let the relay treat a
        // longer run's sequences as duplicates of the previous run's.
        let alex = alex.insert(Client::join(&relay, 1));
        // Receive while talking, so arrival times are real.
        let (talked, received) = std::thread::scope(|scope| {
            let recorder = scope.spawn(|| record(&sam, Duration::from_millis(300)));
            (talk(alex, &mut sender, &input), recorder.join().unwrap())
        });
        let sent = talked.sent;

        let mut inbound = Inbound::new(ROOM_ID, &SPACE_KEY, [(1, alex_identity.verifying_key())]);
        let samples = input.len() + SAMPLE_RATE as usize;
        let (output, mixer, worst_late_us) =
            play(received, &sam, &talked, frame, &mut inbound, samples);

        // Every packet arrived, decrypted, and was played.
        let stats = inbound.stats();
        assert_eq!(stats.audio, sent, "{frame}: {stats:?}");
        assert_eq!(
            (stats.no_key, stats.bad_tag, stats.bad_signature),
            (0, 0, 0),
            "{frame}"
        );
        let track = mixer.stats()[0];
        assert_eq!(track.jitter.played, sent, "{frame}: {track:?}");
        assert_eq!(track.jitter.spurts, 1, "{frame}: {track:?}");
        // With ideal arrivals the buffer aims at its floor: one frame, or
        // two for a redundant sender, from its very first spurt.
        let floor = if redundancy { 2 } else { 1 } * frame.micros();
        assert_eq!(track.jitter.target_us, floor, "{frame}: {track:?}");

        // The output is the input, delayed by the codec and the buffer.
        // Compare the tones, minus 20 ms at each edge where the gate and
        // the codec settle.
        let talk_start = 200 * SAMPLE_RATE as usize / 1000;
        let talk_end = 800 * SAMPLE_RATE as usize / 1000;
        // Search 70 ms of lag on the first 100 ms of tones.
        let probe = talk_start..talk_start + 4800;
        let lag = best_lag(&input[probe.clone()], &output[probe.start..], 3360);
        // The delay is exactly what the latency budget says (spec section
        // 4): the codec's lookahead, one frame to capture it, the jitter
        // buffer's target, and the wait for the playback tick, which starts
        // with the track at the start of a 1 ms card period and so falls
        // anywhere in one frame.
        let frame_samples = frame.samples();
        let target = track.jitter.target_us as usize * SAMPLE_RATE as usize / 1_000_000;
        let earliest = LOOKAHEAD + frame_samples + target;
        assert!(
            (earliest..earliest + frame_samples).contains(&lag),
            "{frame}: lag {lag} samples, expected {earliest} plus less than a frame"
        );
        let edge = 960;
        let snr = snr_db(
            &input[talk_start + edge..talk_end - edge],
            &output[talk_start + edge + lag..talk_end - edge + lag],
        );
        // A concealed frame would stretch the spurt and break the alignment.
        assert_eq!(track.jitter.concealed, 0, "{frame}: {track:?}");
        assert!(
            snr > 15.0,
            "{frame}: SNR {snr:.1} dB at lag {lag} {track:?}"
        );
        last_sent = sent;
        eprintln!(
            "{frame}{}: {sent} packets, SNR {snr:.1} dB at lag {lag}, buffer target {} µs, \
             real arrivals up to {worst_late_us} µs behind schedule",
            if redundancy { " + redundancy" } else { "" },
            track.jitter.target_us
        );
    }

    let mut alex = alex.expect("three runs");
    // A ping comes back with the round trip and the uplink counters of the
    // last run's binding. It is
    // built by hand: the test did not pump the link while talking, and a
    // link without pongs for five seconds says hello again instead.
    alex.socket.send(&new_ping(1, 1, alex.now_us())).unwrap();
    alex.socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let pongs_before = alex.link.stats().pongs;
    while alex.link.stats().pongs == pongs_before {
        alex.receive_control();
    }
    let link = alex.link.stats();
    assert!(link.rtt_us.is_some_and(|rtt| rtt < 1_000_000), "{link:?}");
    assert_eq!(link.uplink_lost, 0, "{link:?}");
    assert_eq!(u64::from(link.uplink_received), last_sent, "{link:?}");

    // A keepalive is accepted silently: nothing comes back, nothing drops.
    alex.socket.send(&new_keepalive(1)).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let status = relay.status();
    let dropped = &status["packets_dropped"];
    assert!(
        dropped.as_object().is_none_or(|d| d.is_empty()),
        "the relay dropped packets: {status}"
    );
}
