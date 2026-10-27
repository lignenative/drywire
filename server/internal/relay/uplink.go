package relay

import (
	"math"

	"github.com/lignenative/drywire/server/internal/wire"
)

// uplink counts the audio packets one session sent to the relay and how many
// never arrived, from sequence gaps (protocol section 10.4). The zero value
// is ready to use and means "nothing received since the last hello".
type uplink struct {
	started bool
	epoch   uint8
	window  wire.ReplayWindow

	// Range of sequences received in the current epoch.
	lowest, highest uint32

	expectedEarlier uint64 // packets expected in epochs already left
	received        uint64 // packets received, duplicates excluded
}

// record notes an audio packet and reports whether it is a duplicate, which
// the relay drops.
func (u *uplink) record(epoch uint8, seq uint32) (duplicate bool) {
	switch {
	case !u.started:
		u.startEpoch(epoch, seq)
	case epoch == u.epoch:
	case epoch == u.epoch+1: // wraps from 255 to 0, like the epoch itself
		u.expectedEarlier += u.expectedInEpoch()
		u.startEpoch(epoch, seq)
	default:
		// A late packet from an older epoch, or an unexpected jump: forward
		// it, but it says nothing reliable about loss.
		return false
	}
	if u.window.Check(seq) == wire.VerdictDuplicate {
		return true
	}
	u.received++
	u.lowest = min(u.lowest, seq)
	u.highest = max(u.highest, seq)
	return false
}

func (u *uplink) startEpoch(epoch uint8, seq uint32) {
	u.started, u.epoch = true, epoch
	u.window = wire.ReplayWindow{}
	u.lowest, u.highest = seq, seq
}

func (u *uplink) expectedInEpoch() uint64 {
	if !u.started {
		return 0
	}
	return uint64(u.highest-u.lowest) + 1
}

func (u *uplink) receivedCount() uint32 {
	return clamp(u.received)
}

func (u *uplink) lostCount() uint32 {
	expected := u.expectedEarlier + u.expectedInEpoch()
	if expected <= u.received {
		return 0
	}
	return clamp(expected - u.received)
}

func clamp(n uint64) uint32 {
	return uint32(min(n, math.MaxUint32))
}
