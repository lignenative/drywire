//! Audio from one client to another through the real Go relay:
//! gate → Opus → seal → UDP → relay → UDP → open → decode.
//!
//! Two clients of the development room "lobby" bind their sessions with
//! `hello`. The first one announces its key, then talks for 600 ms at the
//! pace of a microphone, at each frame size. The second records what it
//! receives. On the way, the first client pings the relay and sends a
//! keepalive, and the relay's own counters must show nothing dropped.
//!
//! Only what does not depend on time is checked here: every packet
//! arrives, decrypts, carries the next frame, and decodes back to the
//! input. Sockets, the relay and the test machine's scheduler decide when
//! packets arrive; a busy CI runner, or Windows's coarse sleeps, change
//! that from run to run. Playback timing and latency are checked on a
//! simulated clock, with no socket, in `simulated_call.rs`.

#[path = "../../testsupport/relay.rs"]
mod relay;

use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use drywire_core::inbound::{Event, Inbound};
use drywire_core::link::{Link, LinkConfig};
use drywire_core::opus::{Decoder, FrameSize, SAMPLE_RATE};
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

/// What [`talk`] did.
struct Talk {
    /// Audio packets sent.
    sent: u64,
    /// The index of the first frame the gate let through.
    first_audio_frame: usize,
}

/// Sends `input` frame by frame at the pace of a real microphone, as well
/// as this machine's sleeps allow, and says when it is done.
fn talk(alex: &Client, sender: &mut Sender, input: &[f32], done: &AtomicBool) -> Talk {
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
            first_audio_frame.get_or_insert(k);
        }
        for p in packets.iter() {
            alex.socket.send(p).unwrap();
        }
    }
    done.store(true, Ordering::Relaxed);
    Talk {
        sent: sender.stats().audio,
        first_audio_frame: first_audio_frame.expect("the gate opened"),
    }
}

/// Records every packet the client receives, until the talker is done and
/// half a second passes without a packet. However long the talker stalls,
/// nothing it sends is missed.
fn record(sam: &Client, done: &AtomicBool) -> Vec<Vec<u8>> {
    sam.socket
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let mut got = Vec::new();
    let mut buf = [0u8; 1500];
    let mut quiet_since = None;
    loop {
        match sam.socket.recv(&mut buf) {
            Ok(n) => {
                got.push(buf[..n].to_vec());
                quiet_since = None;
            }
            Err(_) if done.load(Ordering::Relaxed) => {
                let since = *quiet_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= Duration::from_millis(500) {
                    return got;
                }
            }
            Err(_) => {}
        }
    }
}

/// Opens every packet and decodes the primary frames in timestamp order,
/// as one stream. Returns the decoded samples, and checks that the frames
/// follow each other with no gap: one talk spurt, nothing missing.
fn decode_all(received: &[Vec<u8>], inbound: &mut Inbound, frame: FrameSize) -> Vec<f32> {
    let mut frames: Vec<_> = received
        .iter()
        .filter_map(|packet| match inbound.handle(packet)? {
            Event::Audio { arrival, .. } => Some(arrival),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    let first = frames.first().expect("received audio").timestamp;
    frames.sort_by_key(|a| a.timestamp.wrapping_sub(first));
    let mut decoder = Decoder::new().unwrap();
    let mut output = Vec::new();
    let mut pcm = vec![0.0; frame.samples()];
    for (k, arrival) in frames.iter().enumerate() {
        let expected = first.wrapping_add((k * frame.samples()) as u32);
        assert_eq!(arrival.timestamp, expected, "{frame}: frame {k} is missing");
        decoder.decode(&arrival.primary, &mut pcm).unwrap();
        output.extend_from_slice(&pcm);
    }
    output
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
        let done = AtomicBool::new(false);
        let (talked, received) = std::thread::scope(|scope| {
            let recorder = scope.spawn(|| record(&sam, &done));
            (
                talk(alex, &mut sender, &input, &done),
                recorder.join().unwrap(),
            )
        });
        let sent = talked.sent;

        // Every packet arrived and decrypted, and the frames are complete.
        let mut inbound = Inbound::new(ROOM_ID, &SPACE_KEY, [(1, alex_identity.verifying_key())]);
        let output = decode_all(&received, &mut inbound, frame);
        let stats = inbound.stats();
        assert_eq!(stats.audio, sent, "{frame}: {stats:?}");
        assert_eq!(
            (
                stats.no_key,
                stats.bad_tag,
                stats.bad_signature,
                stats.duplicate
            ),
            (0, 0, 0, 0),
            "{frame}"
        );

        // Decoded, the frames are the input from the frame the gate opened
        // on, delayed by exactly the codec's lookahead. Compare the tones,
        // minus 20 ms at each edge where the gate and the codec settle.
        let start = talked.first_audio_frame * frame.samples();
        let talk_start = 200 * SAMPLE_RATE as usize / 1000;
        let talk_end = 800 * SAMPLE_RATE as usize / 1000;
        let edge = 960;
        let at = |i: usize| i - start + LOOKAHEAD;
        let snr = snr_db(
            &input[talk_start + edge..talk_end - edge],
            &output[at(talk_start + edge)..at(talk_end - edge)],
        );
        assert!(snr > 15.0, "{frame}: SNR {snr:.1} dB");
        last_sent = sent;
        eprintln!(
            "{frame}{}: {sent} packets, SNR {snr:.1} dB",
            if redundancy { " + redundancy" } else { "" },
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
