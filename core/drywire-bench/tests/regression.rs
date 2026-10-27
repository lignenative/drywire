//! Regression bounds for the jitter buffer, checked on every `make test`.
//!
//! Each test runs one built-in scenario at the three frame sizes, with a fixed
//! seed, and checks what must hold. Runs are deterministic, so a failure
//! always means the code changed. The bounds are what the buffer achieved on
//! 2026-10-27 plus some headroom: if a change makes them tighter, tighten the
//! test too; if a change needs them looser, explain why in an ADR.

use drywire_bench::scenarios;
use drywire_bench::simulate::{Report, run};
use drywire_core::jitter::JitterConfig;
use drywire_core::opus::FrameSize;

const SEED: u64 = 1;

/// Runs the named scenario at every frame size.
fn reports(name: &str) -> Vec<Report> {
    FrameSize::ALL
        .into_iter()
        .map(|frame| {
            let scenario = scenarios::all(frame, SEED)
                .into_iter()
                .find(|s| s.name == name)
                .unwrap_or_else(|| panic!("no scenario named {name}"));
            run(&scenario)
        })
        .collect()
}

fn frame_us(r: &Report) -> u64 {
    r.scenario.frame.micros()
}

/// Concealed ticks allowed when the `end_of_spurt` packet itself is lost: the
/// buffer conceals until it gives up on the spurt.
fn max_conceal_frames(r: &Report) -> u64 {
    JitterConfig::new(r.scenario.frame).max_conceal_us / frame_us(r)
}

/// The target delay of the last spurt is at most `bound_us`, or the
/// one-frame floor when that is larger. Without this, a regression that
/// inflates the target would pass `check_added_p95`, which is relative to it.
fn check_target(r: &Report, bound_us: u64) {
    let bound = bound_us.max(frame_us(r));
    assert!(
        r.buffer.target_us <= bound,
        "{}: target {} µs > {bound} µs",
        r.scenario.frame,
        r.buffer.target_us
    );
}

/// The latency added to 95 % of frames, at most `extra_us` above what the
/// target and the tick alignment explain.
fn check_added_p95(r: &Report, extra_us: u64) {
    let bound = r.buffer.target_us + frame_us(r) + extra_us;
    assert!(
        r.added_us[1] <= bound,
        "{}: added p95 {} µs > {bound} µs",
        r.scenario.frame,
        r.added_us[1]
    );
}

#[test]
fn every_scenario_plays_in_order_and_accounts_for_every_frame() {
    for frame in FrameSize::ALL {
        for scenario in scenarios::all(frame, SEED) {
            let r = run(&scenario);
            let name = format!("{} at {frame}", scenario.name);
            assert_eq!(r.out_of_order, 0, "{name}");
            assert_eq!(r.buffer.discontinuities, 0, "{name}: no timeline jump here");
            assert_eq!(r.buffer.received, r.sent - r.lost, "{name}");
            // Every packet received is played, dropped, or discarded as late
            // or as a duplicate; recovered frames come on top, from copies.
            // (A dropped copy would count twice; these scenarios never drop
            // one.)
            let b = r.buffer;
            assert_eq!(
                b.played - b.recovered + b.dropped + b.late + b.duplicates,
                b.received,
                "{name}: {b:?}"
            );
        }
    }
}

#[test]
fn a_clean_network_adds_one_frame_and_loses_nothing() {
    for r in reports("clean") {
        let b = r.buffer;
        assert_eq!(
            (b.concealed, b.dropped, b.late),
            (0, 0, 0),
            "{}",
            r.scenario.frame
        );
        check_target(&r, 0); // the one-frame floor
        check_added_p95(&r, 0);
    }
}

#[test]
fn a_wired_network_conceals_only_the_lost_frames() {
    for r in reports("wired") {
        let b = r.buffer;
        let frame = r.scenario.frame;
        // Exactly the lost frames with this seed; a lost end_of_spurt packet
        // would add up to one give-up's worth of concealment.
        assert!(b.concealed >= r.lost, "{frame}");
        assert!(
            b.concealed <= r.lost + max_conceal_frames(&r),
            "{frame}: {b:?}"
        );
        assert_eq!((b.dropped, b.late), (0, 0), "{frame}");
        check_target(&r, 0);
        check_added_p95(&r, 0);
    }
}

#[test]
fn bursty_loss_is_concealed_frame_for_frame() {
    for r in reports("bursty-loss") {
        let b = r.buffer;
        let frame = r.scenario.frame;
        assert!(
            b.concealed >= r.lost,
            "{frame}: every lost frame is concealed"
        );
        assert!(
            b.concealed <= r.lost * 11 / 10 + max_conceal_frames(&r),
            "{frame}: concealed {} for {} lost",
            b.concealed,
            r.lost
        );
        assert_eq!(b.dropped, 0, "{frame}");
        check_target(&r, 3_000);
        check_added_p95(&r, frame_us(&r));
    }
}

#[test]
fn redundancy_recovers_most_bursty_losses_for_one_more_frame() {
    let without = reports("bursty-loss");
    for (r, plain) in reports("bursty-loss+red").iter().zip(&without) {
        let b = r.buffer;
        let frame = r.scenario.frame;
        assert_eq!(b.target_us, 2 * frame_us(r), "{frame}: two-frame floor");
        assert!(
            b.concealed + b.recovered <= r.lost + 5,
            "{frame}: each lost frame recovered or concealed once: {b:?}"
        );
        assert!(
            b.concealed * 2 < plain.buffer.concealed,
            "{frame}: redundancy should halve concealment, {} vs {}",
            b.concealed,
            plain.buffer.concealed
        );
        check_added_p95(r, 0);
    }
}

#[test]
fn wifi_delay_spikes_stay_within_bounds() {
    for r in reports("wifi") {
        let frame = r.scenario.frame;
        assert!(
            r.concealed_percent() < 1.5,
            "{frame}: {:.2} % concealed",
            r.concealed_percent()
        );
        assert!(
            r.dropped_percent() < 1.0,
            "{frame}: {:.2} % dropped",
            r.dropped_percent()
        );
        assert!(
            r.added_us[1] < 25_000,
            "{frame}: added p95 {} µs",
            r.added_us[1]
        );
        check_target(&r, 8_000);
    }
}

#[test]
fn clock_drift_is_absorbed_with_at_most_a_stretch_per_spurt() {
    for name in ["drift+100ppm", "drift-100ppm"] {
        for r in reports(name) {
            let b = r.buffer;
            let what = format!("{name} at {}", r.scenario.frame);
            assert!(
                b.concealed <= r.lost + 2 * b.spurts,
                "{what}: concealed {} for {} lost in {} spurts",
                b.concealed,
                r.lost,
                b.spurts
            );
            assert_eq!((b.dropped, b.late), (0, 0), "{what}");
            check_target(&r, 0);
            // 100 ppm over a 30-second monologue is 3 ms of drift.
            check_added_p95(&r, 3_000);
        }
    }
}
