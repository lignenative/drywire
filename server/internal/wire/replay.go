package wire

// ReplayWindowSize is how many sequences behind the highest one a
// ReplayWindow remembers (section 8.5).
const ReplayWindowSize = 64

// Verdict is what a ReplayWindow says about a sequence number.
type Verdict uint8

const (
	VerdictNew       Verdict = iota // first time seen: accept
	VerdictDuplicate                // already seen: drop
	VerdictTooOld                   // older than the window can tell: drop
)

// String returns the name used in docs/protocol.md and in the vectors.
func (v Verdict) String() string {
	switch v {
	case VerdictNew:
		return "new"
	case VerdictDuplicate:
		return "duplicate"
	case VerdictTooOld:
		return "too_old"
	}
	return "unknown"
}

// ReplayWindow remembers which of the last 64 sequence numbers were seen.
// The zero value is an empty window, ready to use.
//
// Bit i of seen is set when sequence highest−i was seen; bit 0 is highest
// itself. Sliding the window forward is a left shift.
type ReplayWindow struct {
	started bool
	highest uint32
	seen    uint64
}

// Check returns the verdict for seq and, if it is new, records it.
func (w *ReplayWindow) Check(seq uint32) Verdict {
	if !w.started {
		w.started, w.highest, w.seen = true, seq, 1
		return VerdictNew
	}
	if seq > w.highest {
		shift := seq - w.highest
		if shift >= ReplayWindowSize {
			w.seen = 1
		} else {
			w.seen = w.seen<<shift | 1
		}
		w.highest = seq
		return VerdictNew
	}
	age := w.highest - seq
	if age >= ReplayWindowSize {
		return VerdictTooOld
	}
	bit := uint64(1) << age
	if w.seen&bit != 0 {
		return VerdictDuplicate
	}
	w.seen |= bit
	return VerdictNew
}

// Highest returns the highest sequence recorded and whether any was.
func (w *ReplayWindow) Highest() (uint32, bool) {
	return w.highest, w.started
}
