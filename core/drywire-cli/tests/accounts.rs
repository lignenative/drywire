//! An account follows its person from device to device (ADR 0051),
//! through the real server: a recovery code brings it back on a device
//! that has nothing, a device code adds one more, and neither carries
//! anything the server can read.

#[path = "../../testsupport/relay.rs"]
mod relay;

use std::path::PathBuf;
use std::process::{Command, Output};

use relay::Relay;

/// Every command logs in from 127.0.0.1, and the server paces logins per
/// address (protocol section 13.2), as it would a person typing commands
/// one after the other: give the budget time to refill.
fn settle() {
    std::thread::sleep(std::time::Duration::from_millis(1200));
}

/// Runs `drywire` and returns its output, failing the test if it failed.
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

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn fails(args: &[&str]) -> String {
    settle();
    let out = Command::new(env!("CARGO_BIN_EXE_drywire"))
        .args(args)
        .output()
        .expect("run drywire");
    assert!(!out.status.success(), "drywire {args:?} should have failed");
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn an_account_moves_to_new_devices_with_a_code() {
    let relay = Relay::start();
    let instance = relay.instance();
    let dir =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("accounts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let home = |name: &str| dir.join(name).to_str().unwrap().to_string();
    let (pc, phone, laptop, other) = (home("pc"), home("phone"), home("laptop"), home("other"));

    run(&[
        "create-space",
        "--home",
        &pc,
        "--instance",
        &instance,
        "--name",
        "Team",
        "--display-name",
        "alex",
    ]);
    let key = stdout(&run(&["identity", "--home", &pc]));

    // A recovery code, kept by the person.
    let code = stdout(&run(&["recovery-code", "--home", &pc]));
    assert_eq!(code.len(), 32, "groups of four: {code}");

    // A new space after the code: the vault follows.
    run(&[
        "create-space",
        "--home",
        &pc,
        "--instance",
        &instance,
        "--name",
        "Scrims",
        "--display-name",
        "alex",
    ]);

    // A phone with nothing: the code alone gives it the account and both
    // spaces, keys included.
    let restored = stdout(&run(&[
        "restore",
        &code,
        "--home",
        &phone,
        "--instance",
        &instance,
    ]));
    assert!(restored.contains("2 space(s)"), "{restored}");
    assert_eq!(stdout(&run(&["identity", "--home", &phone])), key);
    let spaces = stdout(&run(&["spaces", "--home", &phone]));
    assert!(
        spaces.contains("Team") && spaces.contains("Scrims"),
        "{spaces}"
    );
    // The phone can invite: it holds the space keys.
    let link = stdout(&run(&[
        "invite", "--home", &phone, "--space", "Team", "--uses", "1",
    ]));
    assert!(link.starts_with("https://drywire.gg/i#"), "{link}");

    // A device code: one new device, once.
    let device = stdout(&run(&["device-code", "--home", &phone]));
    run(&[
        "restore",
        &device,
        "--home",
        &laptop,
        "--instance",
        &instance,
    ]);
    assert_eq!(stdout(&run(&["identity", "--home", &laptop])), key);
    let again = fails(&[
        "restore",
        &device,
        "--home",
        &home("fourth"),
        "--instance",
        &instance,
    ]);
    assert!(again.contains("unknown, used or expired"), "{again}");

    // A device that already is someone else is not replaced by accident.
    run(&["accept", &link, "--home", &other, "--display-name", "sam"]);
    let refused = fails(&["restore", &code, "--home", &other, "--instance", &instance]);
    assert!(refused.contains("--force"), "{refused}");

    // A new recovery code: the old one opens nothing any more.
    let new_code = stdout(&run(&["recovery-code", "--home", &laptop]));
    assert_ne!(new_code, code);
    let old = fails(&[
        "restore",
        &code,
        "--home",
        &home("fifth"),
        "--instance",
        &instance,
    ]);
    assert!(old.contains("unknown, used or expired"), "{old}");
    // A typo is told apart from an unknown code.
    let typo = fails(&[
        "restore",
        "not-a-code",
        "--home",
        &home("sixth"),
        "--instance",
        &instance,
    ]);
    assert!(typo.contains("not a Drywire code"), "{typo}");

    std::fs::remove_dir_all(&dir).unwrap();
}
