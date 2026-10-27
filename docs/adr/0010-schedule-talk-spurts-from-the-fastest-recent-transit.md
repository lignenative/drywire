# 0010. Schedule talk spurts from the fastest recent transit

Status: Accepted · 2026-10-27 · Underrun handling superseded by ADR 0011

## Context

The spec fixes the jitter buffer's policy: one per sender, a target depth of
"p95 of recent jitter plus a margin, never below one frame", adjusted only
between talk spurts, with concealment on underrun and a drop when more than
two frames above target pile up. It leaves open how to measure jitter, how
to order frames, and when exactly a spurt starts playing.

## Decision

- **Frames are ordered by timestamp, not sequence.** The audio sequence
  restarts at 0 with every key epoch (ADR 0002) and does not advance during
  silences, whereas the timestamp advances with the sender's clock in both
  cases. A gap in timestamps is exactly a gap in sound, and an epoch change
  needs no special case. Timestamps are unwrapped to 64 bits.
- **Jitter is measured against the fastest recent transit.** For each
  packet, `transit = arrival time − timestamp position`. Its absolute value
  is meaningless (the two clocks are unrelated), but differences are not:
  the smallest transit among the last 500 packets is the "on time"
  baseline, and each packet's jitter is how far its transit lies above it.
  The target delay is the 95th percentile of those jitters plus a 1 ms
  margin, and at least one frame.
- **A spurt starts at its first frame's on-time arrival plus the target**,
  at the first playback tick at or after that instant. Scheduling from the
  on-time arrival, rather than from when the first packet actually arrived,
  means a first packet that was itself late does not push the whole spurt
  later.
- **Within a spurt the buffer follows the playback clock**, one frame per
  tick. Clock drift between sender and receiver (tens of ppm) only
  accumulates within a spurt, and each silence resets it.
- **A spurt also ends after 60 ms of consecutive concealment**, so a lost
  `end_of_spurt` packet does not leave the buffer concealing forever.
- **A redundant copy fills a missing slot** if its own packet has not
  arrived and its tick has not passed; if the packet arrives afterwards, it
  only contributes its flags.
- The buffer holds opaque frames and takes the time as an argument, so it
  knows nothing about Opus, sockets or clocks, and `drywire-bench` drives it
  with simulated time.

The margin, the history length and the 60 ms limit are starting points:
`drywire-bench` measures their effect, and listening tests will settle
them.

## Consequences

- The buffer adds no latency on top of the target and the tick alignment:
  on a network without jitter, a frame waits exactly one frame plus the
  time to the next tick.
- The first spurt after joining uses the one-frame floor, because there is
  no history yet; it conceals more if the network is jittery. The next
  spurt uses what the first one measured.
- A receiver must create the buffer with the sender's frame size, read
  from the first frame, and create a new one if the sender changes it, or
  after a new announcement shows the sender restarted (`reset`).
