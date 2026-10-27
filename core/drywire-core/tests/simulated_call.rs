//! A call on a simulated clock: playback timing and latency, exactly.
//!
//! The sender, the network and the sound card all run on one clock that
//! the test advances, so nothing here depends on how the machine schedules
//! threads or how coarse its sleeps are. Frame k is captured at
//! (k + 1) frames, its packets cross a network that takes exactly 1 ms, and
//! the receiver's sound card asks for 1 ms (48 samples) at a time:
//!
//! gate → Opus → seal → (1 ms) → open → jitter buffer → decode → mix.
//!
//! The real sockets and relay are checked, without timing, by
//! `relay_round_trip.rs`.

use drywire_core::inbound::Inbound;
use drywire_core::mixer::Mixer;
use drywire_core::opus::{FrameSize, SAMPLE_RATE};
use drywire_core::sender::{Packets, Sender, SenderConfig};
use drywire_proto::crypto::SigningKey;

const ROOM_ID: [u8; 16] = [0x0f; 16];
const SPACE_KEY: [u8; 32] = [0x5a; 32];
/// Opus delays the signal by 2.5 ms (`opus::tests` pins it).
const LOOKAHEAD: usize = 120;
/// The network's one-way delay.
const TRANSIT_US: u64 = 1_000;

/// 200 ms of silence, a 600 ms sweep from 300 Hz to 3 kHz, 400 ms of
/// silence: the gate opens on the sweep and closes during the last silence.
///
/// A sweep, because its frequency never repeats, so it looks like itself at
/// one delay only. Steady tones did not: two tones of 440 and 1234 Hz were
/// 99.9% alike 545 samples apart, and libopus built for Windows tipped the
/// search to that wrong delay.
fn speech() -> Vec<f32> {
    let n = |ms: usize| ms * SAMPLE_RATE as usize / 1000;
    let mut signal = vec![0.0; n(200)];
    let (f0, f1, seconds) = (300.0f64, 3_000.0f64, 0.6f64);
    signal.extend((0..n(600)).map(|i| {
        let t = i as f64 / f64::from(SAMPLE_RATE);
        let phase = 2.0 * std::f64::consts::PI * (f0 * t + (f1 - f0) * t * t / (2.0 * seconds));
        (0.4 * phase.sin()) as f32
    }));
    signal.extend(vec![0.0; n(400)]);
    signal
}

fn us_to_samples(us: u64) -> usize {
    (us * u64::from(SAMPLE_RATE) / 1_000_000) as usize
}

/// How much `output`, delayed by `lag`, looks like `input`.
fn correlation(input: &[f32], output: &[f32], lag: usize) -> f32 {
    input
        .iter()
        .zip(output.get(lag..).unwrap_or(&[]))
        .map(|(a, b)| a * b)
        .sum()
}

/// The delay at which `output` best matches `input`, searched up to
/// `max_lag` samples, and how much better it matches than any delay more
/// than half a millisecond away from it.
fn best_lag(input: &[f32], output: &[f32], max_lag: usize) -> (usize, f32) {
    let scores: Vec<f32> = (0..max_lag)
        .map(|lag| correlation(input, output, lag))
        .collect();
    let best = (0..max_lag)
        .max_by(|a, b| scores[*a].total_cmp(&scores[*b]))
        .unwrap();
    let runner_up = (0..max_lag)
        .filter(|lag| lag.abs_diff(best) > 24)
        .map(|lag| scores[lag])
        .fold(f32::MIN, f32::max);
    (best, scores[best] / runner_up)
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
fn playback_is_on_time_at_every_frame_size() {
    let identity = SigningKey::from_bytes(&[0xa1; 32]);
    let input = speech();

    for (frame, redundancy) in [
        (FrameSize::Ms2_5, true),
        (FrameSize::Ms2_5, false),
        (FrameSize::Ms5, false),
        (FrameSize::Ms10, false),
    ] {
        let config = SenderConfig {
            redundancy,
            ..SenderConfig::new(frame)
        };
        let mut sender = Sender::new(identity.clone(), 1, ROOM_ID, &SPACE_KEY, config).unwrap();
        let mut inbound = Inbound::new(ROOM_ID, &SPACE_KEY, [(1, identity.verifying_key())]);
        let mut mixer = Mixer::new();
        mixer.add_track(1).unwrap();

        // Everything the sender produces, with the time it arrives: the
        // epoch's announcement first, then each frame's packets.
        let mut in_flight: Vec<(u64, Vec<u8>)> = vec![(TRANSIT_US, sender.announce())];
        let mut packets = Packets::new();
        for (k, pcm) in input.chunks_exact(frame.samples()).enumerate() {
            sender.process(pcm, &mut packets).unwrap();
            let ready_us = (k as u64 + 1) * frame.micros();
            in_flight.extend(packets.iter().map(|p| (ready_us + TRANSIT_US, p.to_vec())));
        }
        let sent = sender.stats().audio;

        // The receiver's sound card, 1 ms at a time.
        let mut output = vec![0.0f32; input.len() + SAMPLE_RATE as usize];
        let mut arriving = in_flight.into_iter().peekable();
        for (i, chunk) in output.chunks_mut(48).enumerate() {
            let now = i as u64 * 1_000;
            while let Some((at, packet)) = arriving.next_if(|(at, _)| *at <= now) {
                if let Some(event) = inbound.handle(&packet) {
                    mixer.push(at, event).unwrap();
                }
            }
            mixer.render(now, chunk);
        }

        let label = format!("{frame}{}", if redundancy { " + redundancy" } else { "" });
        assert_eq!(inbound.stats().audio, sent, "{label}");
        let track = mixer.stats()[0];
        let j = track.jitter;
        assert_eq!((j.played, j.spurts), (sent, 1), "{label}: {track:?}");
        assert_eq!(
            (j.concealed, j.dropped, j.late, j.recovered),
            (0, 0, 0, 0),
            "{label}: {track:?}"
        );
        // The buffer aims at its floor: one frame, or two for a redundant
        // sender, from its very first spurt.
        let floor = if redundancy { 2 } else { 1 } * frame.micros();
        assert_eq!(j.target_us, floor, "{label}: {track:?}");

        // The delay is exactly the latency budget of spec section 4: one
        // frame to capture, the network, the buffer's target, and the
        // codec's lookahead. The spurt starts on its schedule: playback
        // asks every 0.5 ms between spurts, and every time here falls on a
        // multiple of 0.5 ms.
        let talk_start = 200 * SAMPLE_RATE as usize / 1000;
        let talk_end = 800 * SAMPLE_RATE as usize / 1000;
        let probe = talk_start..talk_start + 4800;
        let (lag, margin) = best_lag(&input[probe.clone()], &output[probe.start..], 3360);
        // The test signal must make the answer clear-cut, so that no
        // rounding difference between platforms can move it.
        assert!(
            margin > 2.0,
            "{label}: best delay {lag} only {margin:.3} times the next"
        );
        let expected =
            frame.samples() + us_to_samples(TRANSIT_US) + us_to_samples(j.target_us) + LOOKAHEAD;
        // Should it ever fail, say how sure the search was: the match at
        // the expected delay, at the one found, and how alike the input is
        // to itself over the difference.
        let probe_in = &input[probe.clone()];
        let probe_out = &output[probe.start..];
        assert_eq!(
            lag,
            expected,
            "{label}: correlation {:.3} at {expected}, {:.3} at {lag}; input self-similarity \
             {:.3} at 0, {:.3} at {}",
            correlation(probe_in, probe_out, expected),
            correlation(probe_in, probe_out, lag),
            correlation(probe_in, probe_in, 0),
            correlation(probe_in, &input[probe.start..], lag.abs_diff(expected)),
            lag.abs_diff(expected),
        );

        // And what plays is the input. Compare the tones, minus 20 ms at
        // each edge where the gate and the codec settle.
        let edge = 960;
        let snr = snr_db(
            &input[talk_start + edge..talk_end - edge],
            &output[talk_start + edge + lag..talk_end - edge + lag],
        );
        assert!(snr > 15.0, "{label}: SNR {snr:.1} dB at lag {lag}");
    }
}
