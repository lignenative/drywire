# 0011. Wait for a late frame instead of skipping it

Status: Accepted · 2026-10-27

## Context

The first version of the jitter buffer (ADR 0010) handled a missing frame
by concealing it and moving on to the next one. If the frame then arrived,
it was too late and was discarded.

`drywire-bench` showed what this costs. When the sender's clock runs
slower than the receiver's, frames arrive a little later at every tick
within a talk spurt. Once one misses its tick, the next ones miss theirs
too, and each is concealed and then thrown away. With 2.5 ms frames, a
sender 100 ppm slow and a wired path losing 0.1 % of packets, the buffer
concealed 0.87 % of frames and discarded 348 late ones (seed 1, 120
seconds). The same applies after any delay spike that does not drain
before the end of the spurt.

## Decision

- When the frame due at a tick is missing, the buffer conceals one frame
  and **keeps waiting for that same frame**.
- If it arrives, it plays at the next tick: the rest of the spurt is one
  frame later than scheduled. This is how a slow sender clock or a lasting
  delay increase is absorbed within a spurt.
- If a later frame arrives first and its slot has come, the missing frame
  was lost: the buffer skips it, and the concealment already covered its
  slot, so the timing is unchanged.
- A pile-up is still trimmed by dropping one frame per tick, but never a
  frame flagged `end_of_spurt`, since the silence after it resets the delay
  anyway.
- After 60 ms of concealment in a row, the spurt is over, as before.

## Consequences

- Same scenario after the change: 0.09 % concealed, which is the network's
  own loss rate, and no late frame.
- Lossless runs conceal nothing in the bench. A frame that merely misses its
  tick is still concealed once, then plays a tick late; only a lost frame,
  or one more than 60 ms late, is never played.
- The latency added by a stretch lasts until the next silence or until a
  pile-up is trimmed. On the simulated Wi-Fi path, with 30 ms delay spikes,
  this trades concealment for a few dropped frames after each spike.
