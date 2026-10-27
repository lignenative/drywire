# 0012. Start over when a sender's timeline jumps

Status: Accepted · 2026-10-27

## Context

The jitter buffer unwraps 32-bit timestamps by assuming each one is within
2³¹ samples (12.4 hours) of the previous one, and drops any frame at or
before the last one it played (ADR 0010). A review of the first version
found two ordinary situations that break this:

- **A silence longer than 12.4 hours** in a room left open overnight:
  timestamps keep advancing during silence, so the next one looks like it
  went backwards, and every frame was dropped as late, for hours.
- **A sender that restarts**, drawing a new random starting timestamp. If
  it went backwards, the same thing happened; if it went forward, the stale
  transit history pushed the next spurt seconds late. The protocol only
  signals a restart when an announcement replaces a salt for the same
  epoch, which a receiver can miss.

A third consequence: a member sending deliberate timestamp jumps could
grow the unwrapped value until the microsecond arithmetic overflowed.

## Decision

- For every packet, the buffer compares how far the timestamp moved since
  the previous packet with how much time passed between their arrivals.
  Jitter and clock drift make them differ by milliseconds. If they differ by
  more than 10 seconds (`JitterConfig::max_jump_us`), the sender's timeline
  is broken: the buffer forgets its frames and timings, keeps its counters,
  counts a `discontinuity`, and starts over from this packet.
- Receivers still call `reset` when an announcement shows a restart; the
  check makes it a safety net rather than a requirement.

## Consequences

- An overnight silence or a restart costs nothing audible: the next spurt
  is scheduled as if it were the first, with the one-frame floor.
- The unwrapped timestamp can only grow as fast as real time, so the
  arithmetic stays far from overflow.
- A jump under 10 seconds is treated as jitter or as a silence. It is not
  possible with an honest sender, whose timestamps follow its clock.
