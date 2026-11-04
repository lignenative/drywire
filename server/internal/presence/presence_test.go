package presence

import (
	"testing"

	"github.com/lignenative/drywire/server/internal/store"
)

func TestASlowSubscriberIsDroppedNotWaitedFor(t *testing.T) {
	h := NewHub()
	space := store.ID{1}
	sub, snapshot := h.Subscribe(space, store.Key{9})
	if len(snapshot) != 0 {
		t.Fatal(snapshot)
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
	_, snapshot = h.Subscribe(space, store.Key{9})
	if len(snapshot) != eventQueue+1 {
		t.Fatalf("snapshot of %d sessions", len(snapshot))
	}
}

func TestOneMoreStreamEndsTheIdentitysOldest(t *testing.T) {
	h := NewHub()
	var subs []*Subscription
	for range MaxSubscriptions {
		sub, _ := h.Subscribe(store.ID{1}, store.Key{9})
		subs = append(subs, sub)
	}
	ended := func(sub *Subscription) bool {
		select {
		case <-sub.Done():
			return true
		default:
			return false
		}
	}
	// A network change: the client opens a new stream while the old ones
	// still look open here. The oldest goes, the others stay.
	newest, _ := h.Subscribe(store.ID{2}, store.Key{9})
	if !ended(subs[0]) {
		t.Fatal("the oldest stream is still open")
	}
	for i, sub := range subs[1:] {
		if ended(sub) {
			t.Fatalf("stream %d ended too", i+1)
		}
	}
	// Then the next oldest, never the newest.
	h.Subscribe(store.ID{2}, store.Key{9})
	if !ended(subs[1]) || ended(newest) {
		t.Fatal("not the next oldest")
	}
	// Another identity is not touched, and an ended stream gets no events.
	other, _ := h.Subscribe(store.ID{1}, store.Key{8})
	h.Add(Session{Session: 1, Space: store.ID{1}, PublicKey: store.Key{7}})
	if ended(other) || len(other.Events()) != 1 || len(subs[0].Events()) != 0 {
		t.Fatal("another identity's stream was touched, or an ended one still receives")
	}
	open := 0
	for sub := range h.subs {
		if sub.key == (store.Key{9}) {
			open++
		}
	}
	if open != MaxSubscriptions {
		t.Fatalf("%d streams open", open)
	}
}
