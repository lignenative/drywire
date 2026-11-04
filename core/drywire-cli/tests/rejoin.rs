//! A live client joins again by itself when the server forgets its session
//! (ADR 0035, as the apps do): an echo bot survives the server restarting.

#[path = "../../testsupport/relay.rs"]
mod relay;

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use relay::Relay;

fn run(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_drywire"))
        .args(args)
        .output()
        .expect("run drywire");
    assert!(
        out.status.success(),
        "drywire {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn an_echo_bot_joins_again_after_the_server_restarts() {
    let mut relay = Relay::start();
    let instance = relay.instance();
    let dir =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("rejoin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let home = |name: &str| dir.join(name).to_str().unwrap().to_string();
    let (owner, bot) = (home("owner"), home("bot"));
    run(&[
        "create-space",
        "--home",
        &owner,
        "--instance",
        &instance,
        "--name",
        "Team",
        "--display-name",
        "owner",
    ]);
    let link = run(&["invite", "--home", &owner, "--uses", "1"]);

    let mut child = Command::new(env!("CARGO_BIN_EXE_drywire"))
        .args([
            "join",
            "--home",
            &bot,
            "--link",
            &link,
            "--display-name",
            "bot",
            "--echo",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the bot");
    let (tx, lines) = mpsc::channel();
    let stderr = BufReader::new(child.stderr.take().unwrap());
    std::thread::spawn(move || {
        for line in stderr.lines().map_while(Result::ok) {
            eprintln!("bot: {line}");
            if tx.send(line).is_err() {
                return;
            }
        }
    });
    let wait_for = |what: &str, deadline: Duration| {
        let until = Instant::now() + deadline;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            match lines.recv_timeout(left) {
                Ok(line) if line.contains(what) => return line,
                Ok(_) => {}
                Err(_) => panic!("the bot never printed {what:?}"),
            }
        }
    };

    wait_for("joined as", Duration::from_secs(10));
    // Sessions and presence live in the server's memory: a restart forgets
    // them, as a crash or an update would.
    relay.restart();
    let lost = wait_for("joining again", Duration::from_secs(20));
    assert!(lost.contains("ended"), "{lost}");
    wait_for("joined as", Duration::from_secs(20));

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}
