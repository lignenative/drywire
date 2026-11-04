//! The whole app, without its window, against the real server: what the
//! interface does when someone pastes an invitation link, lists their
//! spaces, joins a room and leaves.
//!
//! Sound cards are replaced by in-memory devices (`FileIo`), so no test
//! opens a real one or asks for the microphone.

#[path = "../../../core/testsupport/relay.rs"]
mod relay;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use drywire_control::Client;
use drywire_core::audio::file::{FileIo, Source};
use drywire_core::audio::{AudioError, AudioIo, InputCallback, OutputCallback, StreamInfo};
use drywire_core::opus::SAMPLE_RATE;
use drywire_desktop::app::{Desktop, Devices, Preferences};
use drywire_desktop::session::{CallView, LockState, Notice, Notices, PersonLock};
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
    // Alone, nothing is verified: the lock never shows closed for nothing.
    assert_eq!(view.encryption.state, LockState::Pending);
    assert!(view.encryption.people.is_empty());
    wait_for(&sam, "the relay answers pings", |v| {
        v.bound && v.rtt_ms.is_some()
    });

    // The lab needs the sound card the call holds.
    let busy = sam.calibrate(Duration::from_secs(1)).unwrap_err();
    assert_eq!(busy.code, "busy");

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
    // Alex's signed announcement verified: the lock closes.
    let view = wait_for(&sam, "alex verified", |v| {
        v.encryption.state == LockState::Verified
    });
    let alex_lock = PersonLock {
        public_key: alex_key.clone(),
        verified: true,
        rejected: false,
    };
    assert_eq!(view.encryption.people, [alex_lock]);

    // Sam turns Alex up; Alex mutes, and Sam sees them stop.
    sam.set_volume(&alex_key, 1.5).unwrap();
    wait_for(&sam, "alex louder", |v| v.participants[1].volume == 1.5);
    assert_eq!(alex.toggle_muted(), Some(true), "the global shortcut mutes");
    let view = alex.call().unwrap();
    assert!(view.muted);
    let view = wait_for(&sam, "alex muted", |v| !v.participants[1].speaking);
    assert!(view.participants[1].buffer_ms.is_some(), "{view:#?}");
    let alex_seen = &view.participants[1];
    assert!(alex_seen.counters.received > 0 && alex_seen.counters.played > 0);
    let estimate = alex_seen.estimate_ms.expect("an estimate once alex spoke");
    assert!(estimate >= 5.0, "at least one frame: {estimate}");
    assert!(!view.device_measured);
    wait_for(&sam, "loss measured", |v| {
        v.participants[1].loss_pct.is_some()
    });
    alex.set_gate(-30.0).unwrap();
    assert_eq!(alex.call().unwrap().gate_dbfs, -30.0);
    // Chosen, and still chosen after the session thread's next refresh.
    std::thread::sleep(Duration::from_millis(100));
    assert!(alex.call().unwrap().gate_chosen);
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

#[test]
fn the_overlay_corner_is_one_of_four() {
    let (app, _) = desktop("corner", devices(Vec::new()));
    let prefs = |corner: &str| Preferences {
        mute_shortcut: "Ctrl+Shift+M".into(),
        sound_volume: 0.5,
        overlay: true,
        overlay_corner: corner.into(),
    };
    assert_eq!(app.settings().overlay_corner, "top-left", "the default");
    let saved = app.set_preferences(prefs("bottom-right")).unwrap();
    assert_eq!(saved.overlay_corner, "bottom-right");
    let kept = app.set_preferences(prefs("middle")).unwrap();
    assert_eq!(
        kept.overlay_corner, "bottom-right",
        "an unknown corner changes nothing"
    );
}

/// Polls the call until `done` says so, or fails after `seconds`.
fn wait_long(desktop: &Desktop, what: &str, seconds: u64, done: impl Fn(&CallView) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let view = desktop.call().expect("in a call");
        if done(&view) {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: {view:#?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_call_joins_again_after_the_server_restarts() {
    let mut relay = Relay::start();
    let space_key = [0x5a; 32];
    let owner = Client::new(&relay.instance(), SigningKey::from_bytes(&[0xa1; 32])).unwrap();
    let space = owner.create_space("Team", "general", "owner").unwrap();
    let link = || {
        let (invitation, _) = owner
            .invite(&space.space.id, &space_key, None, Some(1))
            .unwrap();
        invitation.to_link()
    };
    let (sam, _) = desktop("sam-restart", devices(Vec::new()));
    let (alex, _) = desktop("alex-restart", devices(voice()));
    sam.accept(&link(), "sam").unwrap();
    alex.accept(&link(), "alex").unwrap();
    let spaces = sam.spaces().unwrap();
    let (instance, space_id, room) = (&spaces[0].instance, &spaces[0].id, &spaces[0].rooms[0].id);
    sam.join(instance, space_id, room).unwrap();
    alex.join(instance, space_id, room).unwrap();
    let view = wait_for(&sam, "sam hears alex", |v| {
        v.participants.len() == 2 && v.participants[1].speaking
    });
    let alex_key = view.participants[1].public_key.clone();
    sam.set_volume(&alex_key, 1.5).unwrap();
    sam.set_gate(-61.0).unwrap();

    // The server goes down, then comes back: it forgot every session,
    // and both apps find out when their event streams come back.
    relay.stop();
    std::thread::sleep(Duration::from_secs(1));
    relay.start_again();
    wait_long(&sam, "sam reconnecting", 15, |v| v.reconnecting);
    assert!(sam.call().unwrap().ended.is_none());
    // The first attempt comes a second after the loss: a countdown shows
    // until then.
    wait_long(&sam, "the countdown", 5, |v| {
        v.reconnecting && v.retry_in_s.is_some_and(|s| (1..=15).contains(&s))
    });
    // Muted while joining again: it must hold in the new call.
    sam.set_muted(true);
    assert!(sam.call().unwrap().muted);

    // Both join again on their own, and sam hears alex again, with what
    // sam had chosen before.
    wait_long(&sam, "sam back with alex", 30, |v| {
        !v.reconnecting && v.bound && v.participants.len() == 2 && v.participants[1].speaking
    });
    let view = sam.call().unwrap();
    assert!(view.ended.is_none(), "{view:#?}");
    assert_eq!(view.retry_in_s, None, "no countdown in a call");
    assert!(view.muted, "the mute made while reconnecting holds");
    assert_eq!(view.gate_dbfs, -61.0, "same gate");
    assert_eq!(view.participants[1].volume, 1.5, "same volume for alex");

    alex.leave();
    sam.leave();
    let status = relay.status();
    assert_eq!(status["sessions"], 0, "both sessions were freed: {status}");
}

/// An echo device that takes time, as a real one does: what it plays, it
/// hears five periods (12.5 ms) later. With no latency at all, a late
/// wake-up followed by an early one would make an echo look older than
/// its click, and a loopback rightly ignores such an echo.
struct SlowEcho(FileIo);

impl AudioIo for SlowEcho {
    fn start(
        &mut self,
        mut input: InputCallback,
        output: OutputCallback,
    ) -> Result<Vec<StreamInfo>, AudioError> {
        let mut late = VecDeque::from(vec![vec![0.0f32; 120]; 4]);
        let delayed = Box::new(move |samples: &[f32]| {
            late.push_back(samples.to_vec());
            input(&late.pop_front().expect("four buffers"));
        });
        self.0.start(delayed, output)
    }

    fn stop(&mut self) {
        self.0.stop();
    }
}

#[test]
fn the_lab_measures_the_device_and_calibrates_the_gate() {
    let echo: Devices = Arc::new(|| {
        Box::new(|| {
            let device = SlowEcho(FileIo::new(Source::Echo, 120, None));
            Ok(Box::new(device) as Box<dyn AudioIo>)
        })
    });
    let (lab, _) = desktop("lab", echo);
    let measured = lab.loopback_test(Duration::from_secs(4)).unwrap();
    let median = measured.median_ms.expect("the clicks are heard");
    assert!((5.0..50.0).contains(&median), "{measured:?}");
    assert!(measured.delays_ms.len() >= 3, "{measured:?}");
    assert_eq!(measured.devices.len(), 2);

    // Background noise at -60 dBFS: the gate opens 4 dB above it.
    let (quiet, _) = desktop("quiet", devices(vec![0.001; 5 * SAMPLE_RATE as usize]));
    let found = quiet.calibrate(Duration::from_secs(1)).unwrap();
    assert!((found.noise_dbfs - -60.0).abs() < 0.1, "{found:?}");
    assert!((found.gate_dbfs - -56.0).abs() < 0.1, "{found:?}");
    assert!(!found.too_loud);
}

/// A device that counts how many of its kind run at once, and the most
/// that ever did.
struct Counted {
    inner: FileIo,
    running: Arc<Mutex<(usize, usize)>>,
}

impl AudioIo for Counted {
    fn start(
        &mut self,
        input: InputCallback,
        output: OutputCallback,
    ) -> Result<Vec<StreamInfo>, AudioError> {
        let mut running = self.running.lock().unwrap();
        running.0 += 1;
        running.1 = running.1.max(running.0);
        drop(running);
        self.inner.start(input, output)
    }

    fn stop(&mut self) {
        self.inner.stop();
        self.running.lock().unwrap().0 -= 1;
    }
}

#[test]
fn the_meter_hears_the_microphone_out_of_a_call() {
    let running = Arc::new(Mutex::new((0, 0)));
    let counted: Devices = {
        let running = Arc::clone(&running);
        Arc::new(move || {
            let running = Arc::clone(&running);
            Box::new(move || {
                let inner = FileIo::new(Source::Samples(voice()), 120, None);
                Ok(Box::new(Counted { inner, running }) as Box<dyn AudioIo>)
            })
        })
    };
    let (app, _) = desktop("meter", counted);

    // The first request opens the microphone; the voice shows within a
    // few polls, at its level (-13.5 dBFS).
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let level = app.input_level().unwrap();
        if level > -16.0 {
            assert!(level < -12.0, "{level}");
            break;
        }
        assert!(Instant::now() < deadline, "the meter stays at {level}");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(running.lock().unwrap().0, 1, "one sound card open");

    // The lab takes the sound card: the meter closes first.
    app.calibrate(Duration::from_millis(200)).unwrap();
    assert_eq!(*running.lock().unwrap(), (0, 1), "never two at once");

    // Asked again, it opens again; then, unasked, it closes by itself.
    app.input_level().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(running.lock().unwrap().0, 1);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        running.lock().unwrap().0,
        0,
        "closed after a second unasked"
    );
}
