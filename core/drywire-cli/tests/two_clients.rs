//! Two `drywire-cli` processes talk through the real server, headless: WAV
//! files instead of a microphone and speakers, so no test ever opens a
//! sound card or asks for microphone permission.
//!
//! Alex creates a space and prints an invitation link; Sam joins with the
//! link alone. Alex plays three tone bursts at 440, 660 and 990 Hz into
//! the call; Sam records what they hear. Both clients' counters must agree that every
//! packet arrived and decrypted, and Sam's recording must hold the three
//! tones, in order.
//!
//! Both clients pace their audio with real sleeps, which a busy CI runner
//! or Windows's coarse timer stretch at will, so nothing here depends on
//! timing: not how many frames the jitter buffer had to drop or conceal,
//! nor how long a burst lasted. Playback timing is checked exactly on a
//! simulated clock, in drywire-core's `simulated_call.rs`.

#[path = "../../testsupport/relay.rs"]
mod relay;

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use drywire_core::opus::SAMPLE_RATE;
use drywire_core::wav;
use relay::Relay;

const RATE: usize = SAMPLE_RATE as usize;
const TONES: [f32; 3] = [440.0, 660.0, 990.0];
const BURST_MS: usize = 300;

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

#[test]
fn two_clients_talk_through_the_relay_with_wav_files() {
    let relay = Relay::start();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("two-clients-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("alex-in.wav");
    let heard = dir.join("sam-heard.wav");
    let alex_heard = dir.join("alex-heard.wav");
    let signal = bursts();
    wav::write(&input, &signal).unwrap();
    let path = |p: &Path| p.to_str().unwrap().to_string();
    let alex_home = path(&dir.join("alex"));
    let sam_home = path(&dir.join("sam"));

    // Alex creates the space and an invitation; the link is all Sam gets.
    let instance = relay.instance();
    let created = cli(&[
        "create-space",
        "--home",
        &alex_home,
        "--instance",
        &instance,
        "--name",
        "Team",
        "--display-name",
        "alex",
    ])
    .output()
    .unwrap();
    assert!(created.status.success(), "{created:?}");
    let invited = cli(&["invite", "--home", &alex_home, "--uses", "1"])
        .output()
        .unwrap();
    assert!(invited.status.success(), "{invited:?}");
    let link = String::from_utf8(invited.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert!(link.starts_with("https://drywire.gg/i#"), "{link}");

    // Sam listens first, so they are in the room before Alex talks.
    let mut sam = cli(&[
        "join",
        "--home",
        &sam_home,
        "--link",
        &link,
        "--display-name",
        "sam",
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

    let mut alex = cli(&[
        "join",
        "--home",
        &alex_home,
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

    // The network path is exact: over loopback nothing is lost, and every
    // packet decrypts.
    let sent = alex_summary["sent_audio"];
    assert!(sent > 0, "{alex_summary:?}");
    assert_eq!(alex_summary["spurts"], 3, "one spurt per burst");
    assert_eq!(sam_summary["received_audio"], sent, "{sam_summary:?}");
    for key in ["no_key", "bad_tag", "duplicate"] {
        assert_eq!(sam_summary[key], 0, "{key}: {sam_summary:?}");
    }
    // Alex's first announcement, sent the moment they join, can reach Sam
    // before the control plane's event saying who Alex is: then it cannot
    // be verified yet. Alex announces again before the first spurt.
    assert!(sam_summary["bad_signature"] <= 1, "{sam_summary:?}");

    // Every frame received is accounted for: played, dropped by the jitter
    // buffer, too late, or dropped before it because playback stalled. A
    // frame played from a redundant copy and then received late counts
    // twice, hence >=. How the frames split between those depends on
    // timing, so only the sum is checked.
    let s = |key: &str| sam_summary[key];
    assert!(
        s("played") + s("dropped") + s("late") + s("playback_queue_full") >= s("received_audio"),
        "frames unaccounted for: {sam_summary:?}"
    );
    assert!(s("played") > 0, "{sam_summary:?}");
    assert_eq!(alex_summary["uplink_lost"], 0, "{alex_summary:?}");
    assert!(alex_summary["pongs"] > 0, "{alex_summary:?}");

    // Sam heard the three tones, in order: each is loudest, among the
    // three, somewhere after the previous one was. Concealment or dropped
    // frames on a stalled machine change the bursts' edges, not this.
    let recording = wav::read(&heard).unwrap();
    let window = RATE / 20;
    // What a −30 dBFS tone gives over a whole window: anything quieter is
    // not the burst.
    let audible = (0.03 * window as f32 / 2.0).powi(2);
    let loudest_at = |tone: f32| -> usize {
        recording
            .chunks_exact(window)
            .enumerate()
            .filter(|(_, w)| {
                power_at(w, tone) > audible
                    && TONES
                        .iter()
                        .all(|other| power_at(w, tone) >= power_at(w, *other))
            })
            .max_by(|(_, a), (_, b)| power_at(a, tone).total_cmp(&power_at(b, tone)))
            .map(|(i, _)| i)
            .unwrap_or_else(|| panic!("{tone} Hz was never heard"))
    };
    let at: Vec<usize> = TONES.iter().map(|t| loudest_at(*t)).collect();
    assert!(
        at[0] < at[1] && at[1] < at[2],
        "tones heard at windows {at:?}"
    );

    // Alex heard nobody: Sam was silent, so his gate never opened.
    let alex_recording = wav::read(&alex_heard).unwrap();
    assert!(alex_recording.iter().all(|x| *x == 0.0));
    assert_eq!(sam_summary["sent_audio"], 0);

    // Sam's identity and the space key stay on Sam's device, private.
    #[cfg(unix)]
    for file in ["identity.json", "spaces.json"] {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join("sam").join(file))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{file}");
    }
}
