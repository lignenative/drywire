//! Two `drywire-cli` processes talk through the real relay, headless: WAV
//! files instead of a microphone and speakers, so no test ever opens a
//! sound card or asks for microphone permission.
//!
//! Alex plays three tone bursts at 440, 660 and 990 Hz into the call; Sam
//! records what he hears. Sam's recording must contain the three bursts, in
//! order, each about as long as it was sent, and both clients' counters
//! must agree that nothing was lost or dropped on the way.

#[path = "../../testsupport/relay.rs"]
mod relay;

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use drywire_core::opus::SAMPLE_RATE;
use drywire_core::wav;
use relay::{Relay, dev_secret, repo_root};

const RATE: usize = SAMPLE_RATE as usize;
const TONES: [f32; 3] = [440.0, 660.0, 990.0];
const BURST_MS: usize = 300;

fn dev_config(name: &str) -> PathBuf {
    repo_root().join(format!("core/drywire-cli/dev/{name}.json"))
}

fn cli(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_drywire-cli"));
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Half a second of silence, then each tone for 300 ms with 400 ms of
/// silence after it.
fn bursts() -> Vec<f32> {
    let mut signal = vec![0.0; RATE / 2];
    for f in TONES {
        signal.extend(
            (0..BURST_MS * RATE / 1000)
                .map(|i| 0.3 * (2.0 * std::f32::consts::PI * f * i as f32 / RATE as f32).sin()),
        );
        signal.extend(vec![0.0; 400 * RATE / 1000]);
    }
    signal
}

/// Waits for the process to print the line that says it joined.
fn wait_until_joined(child: &mut Child) -> std::thread::JoinHandle<String> {
    let mut stderr = BufReader::new(child.stderr.take().expect("piped"));
    let mut line = String::new();
    loop {
        line.clear();
        assert!(
            stderr.read_line(&mut line).expect("read stderr") > 0,
            "the client exited before joining"
        );
        eprint!("client: {line}");
        if line.starts_with("joined as") {
            break;
        }
    }
    // Keep draining stderr so the client never blocks on a full pipe.
    std::thread::spawn(move || {
        let mut rest = String::new();
        for line in stderr.lines().map_while(Result::ok) {
            eprintln!("client: {line}");
            rest += &line;
            rest.push('\n');
        }
        rest
    })
}

/// The `summary key=value …` line a client prints on stdout when it ends.
fn summary(child: Child) -> HashMap<String, u64> {
    let output = child.wait_with_output().expect("wait for the client");
    assert!(
        output.status.success(),
        "the client failed: {:?}",
        output.status
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8");
    let line = stdout
        .lines()
        .find(|l| l.starts_with("summary "))
        .unwrap_or_else(|| panic!("no summary in {stdout:?}"));
    line.split_whitespace()
        .skip(1)
        .map(|pair| {
            let (k, v) = pair.split_once('=').expect("key=value");
            (k.to_string(), v.parse().expect("a number"))
        })
        .collect()
}

/// Power of `frequency` in `samples` (the Goertzel algorithm: one bin of a
/// Fourier transform).
fn power_at(samples: &[f32], frequency: f32) -> f32 {
    let w = 2.0 * std::f32::consts::PI * frequency / RATE as f32;
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    for &x in samples {
        let s = x + 2.0 * w.cos() * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2
}

/// The stretches of `samples` louder than −30 dBFS, in 5 ms blocks, as
/// (start, end) sample indexes.
fn loud_stretches(samples: &[f32]) -> Vec<(usize, usize)> {
    let block = RATE / 200;
    let mut stretches: Vec<(usize, usize)> = Vec::new();
    for (i, chunk) in samples.chunks(block).enumerate() {
        let rms = (chunk.iter().map(|x| x * x).sum::<f32>() / chunk.len() as f32).sqrt();
        if rms < 0.03 {
            continue;
        }
        let start = i * block;
        match stretches.last_mut() {
            Some(last) if last.1 == start => last.1 = start + chunk.len(),
            _ => stretches.push((start, start + chunk.len())),
        }
    }
    stretches
}

#[test]
fn two_clients_talk_through_the_relay_with_wav_files() {
    let relay = Relay::start();
    let relay_addr = relay.udp.to_string();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("two-clients-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("alex-in.wav");
    let heard = dir.join("sam-heard.wav");
    let alex_heard = dir.join("alex-heard.wav");
    let signal = bursts();
    wav::write(&input, &signal).unwrap();
    let path = |p: &Path| p.to_str().unwrap().to_string();

    // Sam listens first, so he is bound before Alex talks.
    let sam_config = path(&dev_config("sam"));
    let mut sam = cli(&[
        "join",
        "--config",
        &sam_config,
        "--relay",
        &relay_addr,
        "--output",
        &path(&heard),
        "--duration",
        "5",
        // The test checks the path, not this machine's scheduling: a busy
        // machine can delay one packet by more than a frame now and then.
        "--min-buffer",
        "30",
    ])
    .spawn()
    .expect("start sam");
    let sam_log = wait_until_joined(&mut sam);

    let alex_config = path(&dev_config("alex"));
    let mut alex = cli(&[
        "join",
        "--config",
        &alex_config,
        "--relay",
        &relay_addr,
        "--input",
        &path(&input),
        "--output",
        &path(&alex_heard),
        "--frame",
        "2.5",
        "--redundancy",
    ])
    .spawn()
    .expect("start alex");
    let alex_log = wait_until_joined(&mut alex);

    let alex_summary = summary(alex);
    let sam_summary = summary(sam);
    let (_, _) = (alex_log.join(), sam_log.join());

    // Nothing lost or dropped anywhere.
    let sent = alex_summary["sent_audio"];
    assert!(sent > 0, "{alex_summary:?}");
    assert_eq!(alex_summary["spurts"], 3, "one spurt per burst");
    assert_eq!(sam_summary["received_audio"], sent, "{sam_summary:?}");
    assert_eq!(sam_summary["played"], sent, "{sam_summary:?}");
    for key in [
        "no_key",
        "bad_tag",
        "bad_signature",
        "duplicate",
        "late",
        "dropped",
    ] {
        assert_eq!(sam_summary[key], 0, "{key}: {sam_summary:?}");
    }
    assert_eq!(alex_summary["uplink_lost"], 0, "{alex_summary:?}");
    assert!(alex_summary["pongs"] > 0, "{alex_summary:?}");

    // Sam heard three bursts, in order, each about 300 ms long.
    let recording = wav::read(&heard).unwrap();
    let stretches = loud_stretches(&recording);
    assert_eq!(stretches.len(), 3, "loud stretches: {stretches:?}");
    for ((start, end), tone) in stretches.iter().zip(TONES) {
        let ms = (end - start) * 1000 / RATE;
        assert!((280..=330).contains(&ms), "{tone} Hz lasted {ms} ms");
        let middle = &recording[start + (end - start) / 4..end - (end - start) / 4];
        let strongest = TONES
            .iter()
            .copied()
            .max_by(|a, b| power_at(middle, *a).total_cmp(&power_at(middle, *b)))
            .unwrap();
        assert_eq!(strongest, tone, "burst at sample {start}");
    }

    // Alex heard nobody: Sam was silent, so his gate never opened.
    let alex_recording = wav::read(&alex_heard).unwrap();
    assert!(alex_recording.iter().all(|x| *x == 0.0));
    assert_eq!(sam_summary["sent_audio"], 0);

    // The development files are the counterpart of server/rooms.dev.json.
    let rooms: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("server/rooms.dev.json")).unwrap(),
    )
    .unwrap();
    let lobby = &rooms["rooms"][0];
    assert_eq!(lobby["name"], "lobby");
    for (session, name) in [(1u32, "alex"), (2, "sam"), (3, "kim")] {
        let config = drywire_core::config::ClientConfig::load(&dev_config(name)).unwrap();
        assert!(config.dev_only);
        assert_eq!(config.session, session);
        assert_eq!(config.session_secret, dev_secret(session));
        let listed = lobby["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["session"] == session)
            .unwrap();
        assert_eq!(
            listed["secret"],
            drywire_core::config::to_hex(&dev_secret(session))
        );
    }
}
