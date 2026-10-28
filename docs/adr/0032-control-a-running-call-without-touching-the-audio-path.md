# 0032. Control a running call without touching the audio path

Status: Accepted · 2026-10-28

## Context

The apps' compact widget (spec section 10) changes a call while it runs:
mute, the voice gate's threshold, a volume per person from 0 to 200 %,
and it shows an input level meter and who is speaking. Each of these
reaches into the capture or the playback callback, which must never
wait, allocate or free (ADR 0017), and which nothing in the interface
may slow down.

## Decision

- **Mute, gate and level are atomics** shared with the capture callback,
  behind a `CallControls` handle (`Call::controls`). The callback reads
  the mute flag and the packed gate thresholds at each call, and applies
  new thresholds to the sender in place. It stores the loudest frame's
  power with `fetch_max` on the `f32` bits (they sort like the numbers
  when not negative); the app takes it with a swap, a few times a second.
- **Mute feeds the gate silence.** The frames are zeroed before the
  sender, so the gate closes the spurt as it does after any word, with
  its `end_of_spurt` frame: the others hear the last word end, not a
  cut, and no new code path decides what to send. The meter still
  shows what the microphone hears. Muting is not processing: an
  unmuted call never alters a sample.
- **Volumes travel like membership changes**: from any thread to the
  network thread (`CallMembers::set_volume`), then through the existing
  wait-free ring to playback, which multiplies the track's samples,
  gliding to a new volume over a few milliseconds so a slider never
  clicks. The mixer keeps the volumes apart from the tracks, in room
  reserved before the call starts, so a volume set before a member's
  first frame applies when their track is assigned, and setting one
  never grows a list.
- **"Speaking" is the track's own state**: whether its last tick played
  voice (decoded or concealed) rather than idle silence. It reaches the
  app in the statistics the playback callback already publishes.

## Consequences

- A setting takes effect at the capture callback's next call (mute,
  gate) or within the network thread's next wake-up, 50 ms at most
  (volume). Neither waits for, nor makes the audio wait for, the
  interface.
- After mute, up to one hold (150 ms by default) of digital silence is
  still encoded and sent before the spurt ends; it costs a few hundred
  bytes.
- `tests/realtime.rs` changes every control under running callbacks and
  still counts zero allocations.
