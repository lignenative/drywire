//! The built-in scenarios. Each one models a kind of connection we expect
//! players to have. The numbers are plausible, not measured: phase 0's
//! network measurements (`mtr` from each player to the relay) will replace
//! them with recorded traces.

use drywire_core::opus::FrameSize;

use crate::network::{Loss, Path, Spikes, Talk};
use crate::simulate::Scenario;

/// A conversation: short and long sentences, short and long pauses.
const CONVERSATION: Talk = Talk {
    spurt_ms: (400, 4000),
    silence_ms: (200, 2000),
};

/// Long monologues, where clock drift has the most time to accumulate.
const MONOLOGUE: Talk = Talk {
    spurt_ms: (10_000, 30_000),
    silence_ms: (300, 1000),
};

/// Every built-in scenario for one frame size, all with the same seed.
pub fn all(frame: FrameSize, seed: u64) -> Vec<Scenario> {
    let base = Scenario {
        name: "",
        frame,
        seconds: 120,
        path: Path {
            base_delay_us: 15_000,
            jitter_mean_us: 0.0,
            loss: Loss::NONE,
            spikes: None,
        },
        talk: CONVERSATION,
        drift_ppm: 0.0,
        redundancy: false,
        seed,
    };
    let wired = Path {
        jitter_mean_us: 300.0,
        loss: Loss::random(0.001),
        ..base.path
    };
    let wifi = Path {
        jitter_mean_us: 2_000.0,
        loss: Loss {
            in_good: 0.002,
            in_bad: 0.3,
            enter_bad: 0.002,
            leave_bad: 0.2,
        },
        spikes: Some(Spikes {
            every_ms: 5_000,
            extra_us: 30_000,
            drain_ms: 80,
        }),
        ..base.path
    };
    let bursty = Path {
        jitter_mean_us: 500.0,
        loss: Loss {
            in_good: 0.0,
            in_bad: 0.5,
            enter_bad: 0.005,
            leave_bad: 0.3,
        },
        ..base.path
    };

    vec![
        Scenario {
            name: "clean",
            ..base
        },
        Scenario {
            name: "wired",
            path: wired,
            ..base
        },
        Scenario {
            name: "wifi",
            path: wifi,
            ..base
        },
        Scenario {
            name: "bursty-loss",
            path: bursty,
            ..base
        },
        Scenario {
            name: "bursty-loss+red",
            path: bursty,
            redundancy: true,
            ..base
        },
        Scenario {
            name: "drift+100ppm",
            path: wired,
            talk: MONOLOGUE,
            drift_ppm: 100.0,
            ..base
        },
        Scenario {
            name: "drift-100ppm",
            path: wired,
            talk: MONOLOGUE,
            drift_ppm: -100.0,
            ..base
        },
    ]
}
