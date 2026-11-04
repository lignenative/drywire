//! Friends sign up by e-mail and talk (ADRs 0052 and 0055), through the
//! real server: a code by mail, user names, an invitation by user name,
//! and a call whose keys come from the space's MLS group, which the
//! owner's app opens to the newcomer's device while it is in the call.

#[path = "../../testsupport/relay.rs"]
mod relay;

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

use drywire_core::wav;
use relay::Relay;

fn settle() {
    std::thread::sleep(Duration::from_millis(1200));
}

fn run(args: &[&str]) -> Output {
    settle();
    let out = Command::new(env!("CARGO_BIN_EXE_drywire"))
        .args(args)
        .output()
        .expect("run drywire");
    assert!(
        out.status.success(),
        "drywire {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// `drywire signin`, typing the code the server mailed.
fn sign_in(relay: &Relay, home: &str, email: &str) {
    settle();
    let mut child = Command::new(env!("CARGO_BIN_EXE_drywire"))
        .args([
            "signin",
            email,
            "--home",
            home,
            "--instance",
            &relay.instance(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run drywire signin");
    let code = (0..50)
        .find_map(|_| {
            std::thread::sleep(Duration::from_millis(100));
            relay.try_mail_code(email)
        })
        .expect("a code by mail");
    writeln!(child.stdin.take().unwrap(), "{code}").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "signin failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Starts `drywire join` and waits for its "joined as" line.
fn join(args: &[&str]) -> (Child, std::thread::JoinHandle<()>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_drywire"))
        .arg("join")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start drywire join");
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert!(
            stderr.read_line(&mut line).unwrap() > 0,
            "the client exited before joining"
        );
        eprint!("client: {line}");
        if line.starts_with("joined as") {
            break;
        }
    }
    let drain = std::thread::spawn(move || {
        for line in stderr.lines().map_while(Result::ok) {
            eprintln!("client: {line}");
        }
    });
    (child, drain)
}

fn summary(child: Child) -> std::collections::HashMap<String, u64> {
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "the client failed");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let line = stdout
        .lines()
        .find(|l| l.starts_with("summary "))
        .expect("a summary");
    line.split_whitespace()
        .skip(1)
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap();
            (k.to_string(), v.parse().unwrap())
        })
        .collect()
}

#[test]
fn friends_signed_up_by_mail_talk_with_keys_from_their_group() {
    let relay = Relay::start();
    let instance = relay.instance();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("mail-accounts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let home = |name: &str| dir.join(name).to_str().unwrap().to_string();
    let (alex, sam) = (home("alex"), home("sam"));

    sign_in(&relay, &alex, "alex@example.com");
    run(&["username", "alex", "--home", &alex, "--instance", &instance]);
    sign_in(&relay, &sam, "sam@example.com");
    run(&["username", "sam", "--home", &sam, "--instance", &instance]);

    run(&[
        "create-space",
        "--home",
        &alex,
        "--instance",
        &instance,
        "--name",
        "Team",
    ]);
    let invited = run(&["invite", "--home", &alex, "--user", "sam"]);
    assert!(String::from_utf8_lossy(&invited.stdout).contains("Invited sam"));
    // A space with a group invites by user name only: no key in a link.
    settle();
    let link = Command::new(env!("CARGO_BIN_EXE_drywire"))
        .args(["invite", "--home", &alex])
        .output()
        .unwrap();
    assert!(!link.status.success());
    let waiting = run(&["invitations", "--home", &sam, "--instance", &instance]);
    assert!(String::from_utf8_lossy(&waiting.stdout).contains("Team"));
    run(&[
        "accept-invitation",
        "Team",
        "--home",
        &sam,
        "--instance",
        &instance,
    ]);

    // Three tones from Sam.
    let input = dir.join("sam-in.wav");
    let mut signal = vec![0.0f32; 24_000];
    for f in [440.0f32, 660.0, 990.0] {
        signal.extend(
            (0..14_400).map(|i| 0.3 * (2.0 * std::f32::consts::PI * f * i as f32 / 48_000.0).sin()),
        );
        signal.extend(vec![0.0; 19_200]);
    }
    wav::write(&input, &signal).unwrap();
    let heard = dir.join("alex-heard.wav");

    // Alex's app is in the call first: it makes the group, and lets Sam's
    // device in when Sam's asks, while the call runs.
    settle();
    let (alex_call, alex_log) = join(&[
        "--home",
        &alex,
        "--output",
        heard.to_str().unwrap(),
        "--duration",
        "12",
        "--min-buffer",
        "30",
    ]);
    settle();
    let (sam_call, sam_log) = join(&["--home", &sam, "--input", input.to_str().unwrap()]);

    let sam_summary = summary(sam_call);
    let alex_summary = summary(alex_call);
    let _ = (alex_log.join(), sam_log.join());
    let sent = sam_summary["sent_audio"];
    assert!(sent > 0, "{sam_summary:?}");
    assert_eq!(alex_summary["received_audio"], sent, "{alex_summary:?}");
    for key in ["no_key", "bad_tag", "bad_signature"] {
        assert_eq!(alex_summary[key], 0, "{key}: {alex_summary:?}");
    }
    assert!(alex_summary["played"] > 0, "{alex_summary:?}");
}
