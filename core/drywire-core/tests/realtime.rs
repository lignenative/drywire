//! The audio callbacks of a call never allocate or free memory (ADR 0017).
//!
//! This test binary replaces the global allocator with one that counts the
//! allocations and frees made while a thread runs an audio callback. Two
//! calls join the real Go relay. The test thread plays the sound cards of
//! both: every 5 ms it hands alex's capture callback 5 ms of microphone
//! (silence, a tone, silence, so the gate opens and closes), and asks each
//! playback callback for 5 ms to play. Every callback must leave the
//! counter at zero, from the first one on, while sam hears alex.
//!
//! Sam starts alone, as if alex joined the room after them: alex becomes a
//! member of sam's call while it runs, and leaves before the end, so the
//! track handed to playback mid-call and its release are counted too.
//! While alex talks, the controls an app uses change under the callbacks:
//! alex mutes and unmutes, retunes the gate, and sam turns alex's volume
//! up.
//!
//! It sees Rust's allocator only; libopus, the one C library on the path,
//! does not allocate when it encodes or decodes.

#[path = "../../testsupport/relay.rs"]
mod relay;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use drywire_core::call::{Call, CallOptions};
use drywire_core::config::{ClientConfig, Member};
use drywire_core::link::LinkConfig;
use drywire_core::opus::{FrameSize, SAMPLE_RATE};
use drywire_core::sender::SenderConfig;

use relay::{Relay, Team};

/// Allocations and frees made inside a callback.
static IN_CALLBACK_CALLS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

/// The system allocator, counting what a callback asks of it.
struct Counting;

fn note() {
    if IN_CALLBACK.try_with(Cell::get).unwrap_or(false) {
        IN_CALLBACK_CALLS.fetch_add(1, Ordering::Relaxed);
    }
}

// SAFETY: every call goes straight to the system allocator, with the same
// arguments; counting touches no memory it hands out.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        // SAFETY: the caller's promises, passed on.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note();
        // SAFETY: as above.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        // SAFETY: as above.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        // SAFETY: as above.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Runs `f` as an audio callback, and returns how many times it called the
/// allocator.
fn as_callback(f: impl FnOnce()) -> u64 {
    let before = IN_CALLBACK_CALLS.load(Ordering::Relaxed);
    IN_CALLBACK.with(|c| c.set(true));
    f();
    IN_CALLBACK.with(|c| c.set(false));
    IN_CALLBACK_CALLS.load(Ordering::Relaxed) - before
}

/// The call of the `me`-th member of the team, who starts knowing the
/// members `known` (themselves included).
fn config(team: &Team, me: usize, known: &[usize]) -> ClientConfig {
    let seat = &team.seats[me];
    ClientConfig {
        room_id: team.room_id,
        space_key: team.space_key,
        session: seat.session,
        session_secret: seat.secret,
        identity: seat.identity.clone(),
        members: known
            .iter()
            .map(|&i| Member {
                session: team.seats[i].session,
                name: team.seats[i].name.clone(),
                public_key: team.seats[i].identity.verifying_key(),
            })
            .collect(),
    }
}

#[test]
fn the_audio_callbacks_never_allocate() {
    // The counter itself works.
    assert_eq!(
        as_callback(|| drop(vec![1u8])),
        2,
        "one allocation, one free"
    );

    let relay = Relay::start();
    let team = relay.team(&["alex", "sam"]);
    let alex_seat = &team.seats[0];
    let options = |frame, redundancy| CallOptions {
        relay: relay.udp,
        sender: SenderConfig {
            redundancy,
            ..SenderConfig::new(frame)
        },
        link: LinkConfig::default(),
        join_timeout: Duration::from_secs(5),
        min_buffer_us: 0,
    };
    // Alex at 2.5 ms with redundancy, so the copy path runs too.
    let (alex, mut alex_capture, mut alex_playback) =
        Call::join(&config(&team, 0, &[0, 1]), options(FrameSize::Ms2_5, true)).unwrap();
    let (sam, mut sam_capture, mut sam_playback) =
        Call::join(&config(&team, 1, &[1]), options(FrameSize::Ms5, false)).unwrap();

    let period = 240; // 5 ms
    let tick = Duration::from_micros(5_000);
    let silence = vec![0.0f32; period];
    let mut out = vec![0.0f32; period];
    let mut phase = 0usize;
    let mut tone = vec![0.0f32; period];
    let started = Instant::now();
    let mut calls = 0u64;
    let mut heard = None;
    let mic = alex.controls();
    let mut sent_when_muted = 0;
    // 200 ms of silence, 600 ms of tone, 400 ms of silence. Alex is in
    // sam's call from 100 ms to 1.1 s.
    for k in 0..240u32 {
        match k {
            20 => sam.add_member(alex_seat.session, alex_seat.identity.verifying_key()),
            30 => mic.set_gate(-45.0, -50.0),
            60 => sam.members().set_volume(alex_seat.session, 1.5),
            // Muted halfway through the tone, for long enough that the
            // hold (150 ms) runs out; unmuted before it ends.
            80 => mic.set_muted(true),
            130 => {
                sent_when_muted = alex.stats().sender.audio;
                assert!(
                    mic.take_input_peak_dbfs() > -20.0,
                    "the meter still hears a muted microphone"
                );
            }
            150 => {
                let sent = alex.stats().sender.audio;
                assert_eq!(sent, sent_when_muted, "nothing is sent while muted");
                mic.set_muted(false);
            }
            220 => {
                heard = sam.stats().tracks.first().copied();
                sam.remove_member(alex_seat.session);
            }
            _ => {}
        }
        std::thread::sleep((started + tick * (k + 1)).saturating_duration_since(Instant::now()));
        let talking = (40..160).contains(&k);
        for x in &mut tone {
            let t = phase as f32 / SAMPLE_RATE as f32;
            *x = 0.3 * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
            phase += 1;
        }
        let input = if talking { &tone } else { &silence };
        calls += as_callback(|| alex_capture(input));
        calls += as_callback(|| sam_capture(&silence));
        calls += as_callback(|| alex_playback(&mut out));
        calls += as_callback(|| sam_playback(&mut out));
        assert_eq!(calls, 0, "a callback used the allocator at tick {k}");
    }

    // The callbacks really did the work: alex's capture sent the spurt, and
    // sam's playback received and played some of it. How much it played
    // depends on how this machine paced the loop, so only "some" is
    // checked; the allocation count above is exact either way.
    drop((alex_capture, alex_playback, sam_capture, sam_playback));
    let sent = alex.stats().sender;
    assert!(sent.audio > 100, "{sent:?}");
    assert!(sent.audio > sent_when_muted, "unmuted, alex talks again");
    assert_eq!(sent.spurts, 2, "muting ended the first spurt: {sent:?}");
    assert_eq!(mic.gate(), (-45.0, -50.0));
    let track = heard.expect("alex's track, while alex was a member");
    assert_eq!(track.session, alex_seat.session);
    assert!(track.jitter.received > 0, "{track:?}");
    assert!(track.jitter.played > 0, "{track:?}");
    // Once alex left, their track was free again. `remove_member` only
    // queues the change: the network thread applies it and then publishes
    // the stats, so on a loaded machine the very next snapshot can still
    // show the track. Wait for it; the deadline only stops a hang, it is
    // not a timing assertion.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sam.stats().tracks.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let left = sam.stats().tracks;
    assert!(left.is_empty(), "{left:?}");
}
