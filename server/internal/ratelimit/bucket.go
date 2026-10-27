// Package ratelimit provides the token bucket the relay uses to cap packets
// and bytes per session and per IP address.
package ratelimit

import "time"

// Bucket is a token bucket: it holds up to Burst tokens and refills at Rate
// tokens per second. Each event spends tokens; an event that would overdraw
// the bucket is refused and spends nothing.
//
// Time is passed in rather than read from the clock, so callers decide what
// "now" is and tests can control it.
type Bucket struct {
	Rate  float64 // tokens added per second
	Burst float64 // capacity

	tokens float64
	last   time.Time
}

// New returns a full bucket.
func New(rate, burst float64, now time.Time) Bucket {
	return Bucket{Rate: rate, Burst: burst, tokens: burst, last: now}
}

// Allow refills the bucket for the time elapsed since the last call, then
// spends cost tokens if there are enough.
func (b *Bucket) Allow(now time.Time, cost float64) bool {
	if elapsed := now.Sub(b.last).Seconds(); elapsed > 0 {
		b.tokens = min(b.Burst, b.tokens+elapsed*b.Rate)
		b.last = now
	}
	if b.tokens < cost {
		return false
	}
	b.tokens -= cost
	return true
}

// Full reports whether the bucket has refilled completely, which means the
// caller can forget it without changing any future decision.
func (b *Bucket) Full(now time.Time) bool {
	elapsed := now.Sub(b.last).Seconds()
	return b.tokens+elapsed*b.Rate >= b.Burst
}
