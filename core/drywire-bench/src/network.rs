//! A simulated network path and a simulated talker, both seeded so that every
//! run of a scenario produces exactly the same trace.

/// A small deterministic random number generator (SplitMix64). Good enough
/// for simulation, and dependency-free; never use it for keys.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// A generator that always produces the same sequence for `seed`.
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A uniform value in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    /// A uniform integer in `[low, high]`.
    pub fn between(&mut self, low: u64, high: u64) -> u64 {
        low + (self.unit() * (high - low + 1) as f64) as u64
    }

    /// An exponentially distributed value with the given mean: most values
    /// small, a long tail of large ones, like queueing delay.
    pub fn exponential(&mut self, mean: f64) -> f64 {
        -mean * (1.0 - self.unit()).ln()
    }
}

/// Packet loss as a two-state Gilbert-Elliott model: a "good" state with
/// rare losses and a "bad" state with frequent ones, which produces the
/// bursts of loss real networks have.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loss {
    /// Probability of losing a packet in the good state.
    pub in_good: f64,
    /// Probability of losing a packet in the bad state.
    pub in_bad: f64,
    /// Probability, per packet, of moving from good to bad.
    pub enter_bad: f64,
    /// Probability, per packet, of moving from bad back to good.
    pub leave_bad: f64,
}

impl Loss {
    /// No loss at all.
    pub const NONE: Loss = Loss::random(0.0);

    /// Independent losses with probability `p`.
    pub const fn random(p: f64) -> Loss {
        Loss {
            in_good: p,
            in_bad: p,
            enter_bad: 0.0,
            leave_bad: 1.0,
        }
    }
}

/// Recurring delay spikes, like a Wi-Fi scan or a bufferbloated uplink: the
/// path suddenly holds packets for `extra_us`, then drains over `drain_ms`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spikes {
    /// Time between the starts of two spikes.
    pub every_ms: u64,
    /// Extra delay at the start of a spike.
    pub extra_us: u64,
    /// How long the extra delay takes to fall back to zero.
    pub drain_ms: u64,
}

/// One network path from a sender to a receiver, through the relay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Path {
    /// Fixed one-way delay.
    pub base_delay_us: u64,
    /// Mean of the exponential jitter added to every packet.
    pub jitter_mean_us: f64,
    /// Packet loss.
    pub loss: Loss,
    /// Delay spikes, if any.
    pub spikes: Option<Spikes>,
}

/// The state of a [`Path`] while packets cross it.
pub struct PathState {
    path: Path,
    bad: bool,
}

impl PathState {
    /// A path in its good state.
    pub fn new(path: Path) -> PathState {
        PathState { path, bad: false }
    }

    /// When a packet sent at `sent_us` arrives, or `None` if it is lost.
    pub fn cross(&mut self, rng: &mut Rng, sent_us: u64) -> Option<u64> {
        let loss = self.path.loss;
        self.bad = if self.bad {
            !rng.chance(loss.leave_bad)
        } else {
            rng.chance(loss.enter_bad)
        };
        if rng.chance(if self.bad { loss.in_bad } else { loss.in_good }) {
            return None;
        }
        let jitter = rng.exponential(self.path.jitter_mean_us) as u64;
        Some(sent_us + self.path.base_delay_us + jitter + self.spike_at(sent_us))
    }

    fn spike_at(&self, sent_us: u64) -> u64 {
        let Some(s) = self.path.spikes else { return 0 };
        let into = sent_us % (s.every_ms * 1000);
        let drain = s.drain_ms * 1000;
        if into >= drain {
            return 0;
        }
        s.extra_us * (drain - into) / drain
    }
}

/// How the simulated person talks: spurts and silences of random lengths.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Talk {
    /// Shortest and longest talk spurt, in milliseconds.
    pub spurt_ms: (u64, u64),
    /// Shortest and longest silence, in milliseconds.
    pub silence_ms: (u64, u64),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_numbers() {
        let a: Vec<u64> = {
            let mut r = Rng::new(7);
            (0..5).map(|_| r.next_u64()).collect()
        };
        let mut r = Rng::new(7);
        assert_eq!(a, (0..5).map(|_| r.next_u64()).collect::<Vec<_>>());
    }

    #[test]
    fn exponential_has_the_requested_mean() {
        let mut r = Rng::new(1);
        let n = 100_000;
        let mean = (0..n).map(|_| r.exponential(2000.0)).sum::<f64>() / n as f64;
        assert!((mean - 2000.0).abs() < 40.0, "{mean}");
    }

    #[test]
    fn gilbert_elliott_loses_in_bursts() {
        let mut path = PathState::new(Path {
            base_delay_us: 0,
            jitter_mean_us: 0.0,
            loss: Loss {
                in_good: 0.0,
                in_bad: 1.0,
                enter_bad: 0.01,
                leave_bad: 0.25,
            },
            spikes: None,
        });
        let mut r = Rng::new(3);
        let lost: Vec<bool> = (0..100_000)
            .map(|_| path.cross(&mut r, 0).is_none())
            .collect();
        let losses = lost.iter().filter(|l| **l).count();
        let bursts = lost.windows(2).filter(|w| !w[0] && w[1]).count();
        // Mean burst length is 1 / leave_bad = 4 packets.
        let mean_burst = losses as f64 / bursts as f64;
        assert!((mean_burst - 4.0).abs() < 0.3, "{mean_burst}");
    }

    #[test]
    fn spikes_drain_linearly() {
        let path = PathState::new(Path {
            base_delay_us: 0,
            jitter_mean_us: 0.0,
            loss: Loss::NONE,
            spikes: Some(Spikes {
                every_ms: 1000,
                extra_us: 40_000,
                drain_ms: 100,
            }),
        });
        assert_eq!(path.spike_at(0), 40_000);
        assert_eq!(path.spike_at(50_000), 20_000);
        assert_eq!(path.spike_at(100_000), 0);
        assert_eq!(path.spike_at(1_000_000), 40_000);
    }
}
