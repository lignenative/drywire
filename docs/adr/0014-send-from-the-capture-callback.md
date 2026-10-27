# 0014. Send from the capture callback

Status: Accepted · 2026-10-27

## Context

A call has three jobs that run at different paces: capturing and sending
(driven by the microphone's clock), receiving and decrypting (driven by the
network), and mixing and playing (driven by the speaker's clock). Where
each job runs decides how many thread hops a word takes, and what the
real-time audio threads might wait on.

The usual design hands captured samples to a network thread through a
queue. Each hop costs a wake-up, tens to hundreds of microseconds, and the
scheduler may add more when the machine is busy.

## Decision

- **Capture callback:** frames the samples, runs the gate, Opus and
  AES-GCM, and sends the packets on the UDP socket itself. Encoding a
  frame takes tens of microseconds; a non-blocking UDP send is a system call
  that returns at once. Sending from here removes the only hop on the send
  path.
- **Network thread:** receives, verifies announcements (Ed25519 is the one
  expensive operation), decrypts, applies the replay window, and passes
  decrypted frames with their arrival time to playback through a channel.
  It also sends pings and keepalives, and says hello again if the relay
  stops answering.
- **Playback callback:** drains the channel into the mixer (jitter buffers
  and decoders) and renders. Decoding happens just before playing, so the
  jitter buffer sees the speaker's real clock.
- The audio callbacks never block on a lock: they publish statistics with
  `try_lock` and skip an update when the lock is busy.

## Consequences

- The send path has no queue at all; the receive path has one hop, from
  the network thread to the playback callback, which the jitter buffer
  absorbs anyway.
- The callbacks still allocate a little (Opus output, sealed packets,
  decrypted frames). That is acceptable for phase 1; if measurements show
  glitches, buffers can be pooled without changing the design.
- The same split serves every client: the native apps hand the two
  callbacks to their platform's audio backend, as `drywire-cli` does.
