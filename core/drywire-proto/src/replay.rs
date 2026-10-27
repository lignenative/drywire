//! The replay window (section 8.5): drops audio packets already seen.

/// How many sequences behind the highest one a [`ReplayWindow`] remembers.
pub const WINDOW_SIZE: u32 = 64;

/// What a [`ReplayWindow`] says about a sequence number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// First time seen: accept.
    New,
    /// Already seen: drop.
    Duplicate,
    /// Older than the window can tell: drop.
    TooOld,
}

impl Verdict {
    /// The name used in `docs/protocol.md` and in the test vectors.
    pub fn name(self) -> &'static str {
        match self {
            Verdict::New => "new",
            Verdict::Duplicate => "duplicate",
            Verdict::TooOld => "too_old",
        }
    }
}

/// Remembers which of the last 64 sequence numbers were seen. Keep one per
/// sender and key epoch, and check a packet only after it decrypted, so
/// forged packets cannot move the window.
///
/// Bit `i` of `seen` is set when sequence `highest − i` was seen; bit 0 is
/// `highest` itself. Sliding the window forward is a left shift.
#[derive(Clone, Debug, Default)]
pub struct ReplayWindow {
    highest: Option<u32>,
    seen: u64,
}

impl ReplayWindow {
    /// An empty window.
    pub fn new() -> ReplayWindow {
        ReplayWindow::default()
    }

    /// Returns the verdict for `sequence` and, if it is new, records it.
    pub fn check(&mut self, sequence: u32) -> Verdict {
        let Some(highest) = self.highest else {
            self.highest = Some(sequence);
            self.seen = 1;
            return Verdict::New;
        };
        if sequence > highest {
            let shift = sequence - highest;
            self.seen = if shift >= WINDOW_SIZE {
                1
            } else {
                self.seen << shift | 1
            };
            self.highest = Some(sequence);
            return Verdict::New;
        }
        let age = highest - sequence;
        if age >= WINDOW_SIZE {
            return Verdict::TooOld;
        }
        let bit = 1u64 << age;
        if self.seen & bit != 0 {
            return Verdict::Duplicate;
        }
        self.seen |= bit;
        Verdict::New
    }

    /// The highest sequence recorded, if any.
    pub fn highest(&self) -> Option<u32> {
        self.highest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_edge_is_exactly_64_wide() {
        let mut w = ReplayWindow::new();
        assert_eq!(w.check(100), Verdict::New);
        assert_eq!(w.check(37), Verdict::New); // 100 − 63, the oldest it still knows
        assert_eq!(w.check(36), Verdict::TooOld);
        assert_eq!(w.check(37), Verdict::Duplicate);
        assert_eq!(w.highest(), Some(100));
    }
}
