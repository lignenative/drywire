package ratelimit

import (
	"testing"
	"time"
)

func TestBucket(t *testing.T) {
	start := time.Unix(1000, 0)
	b := New(10, 5, start) // 10 per second, bursts of 5

	for i := range 5 {
		if !b.Allow(start, 1) {
			t.Fatalf("event %d of the burst was refused", i)
		}
	}
	if b.Allow(start, 1) {
		t.Fatal("a sixth event at the same instant was allowed")
	}

	// 100 ms later, one token is back.
	later := start.Add(100 * time.Millisecond)
	if !b.Allow(later, 1) {
		t.Fatal("the refilled token was refused")
	}
	if b.Allow(later, 1) {
		t.Fatal("more than the refilled token was allowed")
	}

	// A refused event spends nothing: a cost larger than the burst is
	// always refused, and smaller events still pass afterwards.
	muchLater := start.Add(time.Hour)
	if b.Allow(muchLater, 6) {
		t.Fatal("an event costing more than the burst was allowed")
	}
	if !b.Full(muchLater) || !b.Allow(muchLater, 5) {
		t.Fatal("the bucket did not refill to its burst")
	}
}
