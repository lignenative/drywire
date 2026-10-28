//! The whole app, without its window, against the real server: what the
//! interface does when someone pastes an invitation link, lists their
//! spaces, joins a room and leaves.
//!
//! Sound cards are replaced by in-memory devices (`FileIo`), so no test
//! opens a real one or asks for the microphone.

#[path = "../../../core/testsupport/relay.rs"]
mod relay;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use drywire_control::Client;
use drywire_core::audio::AudioIo;
use drywire_core::audio::file::{FileIo, Source};
use drywire_core::opus::SAMPLE_RATE;
use drywire_desktop::app::{Desktop, Devices};
use drywire_desktop::session::{CallView, Notice, Notices};
use drywire_desktop::store::Store;
use drywire_proto::crypto::SigningKey;
use relay::Relay;

/// Devices whose microphone hears `samples`, then silence.
fn devices(samples: Vec<f32>) -> Devices {
    Arc::new(move || {
        let samples = samples.clone();
        Box::new(move || {
            Ok(Box::new(FileIo::new(Source::Samples(samples), 120, None)) as Box<dyn AudioIo>)
        })
    })
}

/// Ten seconds of a 440 Hz tone at −13.5 dBFS: someone talking.
fn voice() -> Vec<f32> {
    let rate = SAMPLE_RATE as f32;
    (0..10 * SAMPLE_RATE as usize)
        .map(|i| 0.3 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate).sin())
        .collect()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("desktop-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The notices an app received, as "kind name".
type Heard = Arc<Mutex<Vec<String>>>;

fn desktop(name: &str, devices: Devices) -> (Desktop, Heard) {
    let heard = Heard::default();
    let notices: Notices = {
        let heard = Arc::clone(&heard);
        Arc::new(move |n: Notice| heard.lock().unwrap().push(format!("{} {}", n.kind, n.name)))
    };
    let app = Desktop::new(Store::open(&temp_dir(name)).unwrap(), devices, notices);
    (app, heard)
}

/// Polls the call until `done` says so, or fails after five seconds.
fn wait_for(desktop: &Desktop, what: &str, done: impl Fn(&CallView) -> bool) -> CallView {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let view = desktop.call().expect("in a call");
        if done(&view) {
            return view;
        }
        assert!(Instant::now() < deadline, "{what}: {view:#?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn names(view: &CallView) -> Vec<&str> {
    view.participants.iter().map(|p| p.name.as_str()).collect()
}

#[test]
fn members_join_by_link_and_meet_in_a_room() {
    let relay = Relay::start();
    let space_key = [0x5a; 32];

    // The owner made a space with another tool, and invites two people.
    let owner = Client::new(&relay.instance(), SigningKey::from_bytes(&[0xa1; 32])).unwrap();
    let space = owner.create_space("Team", "general", "owner").unwrap();
    let link = || {
        let (invitation, _) = owner
            .invite(&space.space.id, &space_key, None, Some(1))
            .unwrap();
        invitation.to_link()
    };

    // Sam's app: a device key, then the link.
    let (sam, sam_heard) = desktop("sam", devices(Vec::new()));
    assert_eq!(sam.toggle_muted(), None, "nothing to mute out of a call");
    let key = sam.public_key().unwrap();
    assert_eq!(key.len(), 64);
    assert_eq!(sam.public_key().unwrap(), key, "the identity is kept");
    let err = sam
        .accept("https://voice.example.org/i/#nope", "sam")
        .unwrap_err();
    assert_eq!(err.code, "invalid_link");
    let accepted = sam.accept(&link(), "sam").unwrap();
    assert_eq!(accepted.name, "Team");

    let spaces = sam.spaces().unwrap();
    assert_eq!(spaces.len(), 1);
    assert_eq!(spaces[0].role, "member");
    assert!(spaces[0].error.is_none());
    let (instance, space_id) = (&spaces[0].instance, &spaces[0].id);
    let room = spaces[0].rooms[0].clone();
    assert_eq!(room.name, "general");

    // Sam joins first, alone.
    let view = sam.join(instance, space_id, &room.id).unwrap();
    assert_eq!(view.room_name, "general");
    assert_eq!(names(&view), ["sam"]);
    assert!(view.participants[0].me);
    assert_eq!(view.devices.len(), 2, "both streams are shown");
    wait_for(&sam, "the relay answers pings", |v| {
        v.bound && v.rtt_ms.is_some()
    });

    // Alex's app joins the same room: each sees the other.
    let (alex, _) = desktop("alex", devices(voice()));
    alex.accept(&link(), "alex").unwrap();
    alex.join(instance, space_id, &room.id).unwrap();
    let view = wait_for(&sam, "alex arrives", |v| v.participants.len() == 2);
    assert_eq!(names(&view), ["sam", "alex"], "this device first");
    let view = wait_for(&alex, "alex sees sam", |v| v.participants.len() == 2);
    assert_eq!(names(&view), ["alex", "sam"]);

    // Alex talks: their meter moves, and Sam sees them speak.
    wait_for(&alex, "alex's meter", |v| {
        v.input_dbfs > -20.0 && v.participants[0].speaking
    });
    let view = wait_for(&sam, "sam hears alex", |v| v.participants[1].speaking);
    let alex_key = view.participants[1].public_key.clone();

    // Sam turns Alex up; Alex mutes, and Sam sees them stop.
    sam.set_volume(&alex_key, 1.5).unwrap();
    wait_for(&sam, "alex louder", |v| v.participants[1].volume == 1.5);
    assert_eq!(alex.toggle_muted(), Some(true), "the global shortcut mutes");
    let view = alex.call().unwrap();
    assert!(view.muted);
    let view = wait_for(&sam, "alex muted", |v| !v.participants[1].speaking);
    assert!(view.participants[1].buffer_ms.is_some(), "{view:#?}");
    wait_for(&sam, "loss measured", |v| {
        v.participants[1].loss_pct.is_some()
    });
    alex.set_gate(-30.0).unwrap();
    assert_eq!(alex.call().unwrap().gate_dbfs, -30.0);
    alex.set_gate(-500.0).unwrap();
    assert_eq!(alex.call().unwrap().gate_dbfs, -80.0, "clamped");

    // Alex leaves: Sam sees them go.
    alex.leave();
    assert!(alex.call().is_none());
    wait_for(&sam, "alex leaves", |v| v.participants.len() == 1);
    assert_eq!(*sam_heard.lock().unwrap(), ["join alex", "leave alex"]);

    // Volumes follow the person into the next call.
    sam.leave();
    assert!(sam.call().is_none());
    alex.join(instance, space_id, &room.id).unwrap();
    sam.join(instance, space_id, &room.id).unwrap();
    wait_for(&sam, "alex keeps their volume", |v| {
        v.participants.len() == 2 && v.participants[1].volume == 1.5
    });
    alex.leave();
    sam.leave();
    let status = relay.status();
    assert_eq!(status["sessions"], 0, "both sessions were freed: {status}");
}
