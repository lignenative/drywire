package presence

import (
	"testing"

	"github.com/lignenative/drywire/server/internal/store"
)

func TestASlowSubscriberIsDroppedNotWaitedFor(t *testing.T) {
	h := NewHub()
	space := store.ID{1}
	sub, snapshot, err := h.Subscribe(space, store.Key{9})
	if err != nil || len(snapshot) != 0 {
		t.Fatal(snapshot, err)
	}
	for i := range eventQueue + 1 {
		h.Add(Session{Session: uint32(i + 1), Space: space, PublicKey: store.Key{byte(i), byte(i >> 8), 1}})
	}
	select {
	case <-sub.Done():
	default:
		t.Fatal("a subscriber that fell behind is still subscribed")
	}
	// Its client reconnects and finds everyone in the snapshot.
	_, snapshot, _ = h.Subscribe(space, store.Key{9})
	if len(snapshot) != eventQueue+1 {
		t.Fatalf("snapshot of %d sessions", len(snapshot))
	}
}

func TestOneIdentityHoldsFewStreams(t *testing.T) {
	h := NewHub()
	for range MaxSubscriptions {
		if _, _, err := h.Subscribe(store.ID{1}, store.Key{9}); err != nil {
			t.Fatal(err)
		}
	}
	if _, _, err := h.Subscribe(store.ID{2}, store.Key{9}); err != ErrTooManySubscriptions {
		t.Fatalf("one more: %v", err)
	}
	if _, _, err := h.Subscribe(store.ID{1}, store.Key{8}); err != nil {
		t.Fatalf("another identity: %v", err)
	}
}
