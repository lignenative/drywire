# 0023. Hand playback a ready track for a member who joins mid-call

Status: Accepted · 2026-10-28

## Context

ADR 0017 keeps the audio callbacks free of allocation, and in phase 1 made
every member's track (an Opus decoder and a jitter buffer) before playback
started, since the room's members were fixed by a file. In phase 2 people
join and leave a room while a call runs: the control plane's event stream
says so (ADR 0021). A track made on the playback thread would allocate
there; a track dropped there would free memory there.

## Decision

- The mixer keeps its tracks in a list with room reserved for 64 (more
  than a room's 31 other members), each boxed.
- When someone joins, the call's network thread, where allocating is fine,
  makes a **free track** (`Mixer::make_slot`) and passes it to playback
  through the same wait-free ring as the frames. Playback pushes the box
  into the list it reserved: no allocation, no copy.
- When someone leaves, the network thread sends `Event::Left`. Playback
  marks their track free and resets it in place (jitter buffer, Opus state
  with `OPUS_RESET_STATE`, counters). The next new sender takes it. Tracks
  are never freed while the call lasts, so a long evening of comings and
  goings uses as many tracks as the most people present at once.
- The network thread counts members and tracks, and makes a new one only
  when every track is in use. A message that finds the ring full waits on
  the network thread and goes first next time, so playback is never a
  track short.
- `tests/realtime.rs` now adds a member to a running call and removes it,
  under the counting allocator.

## Consequences

- A member's audio can be played as soon as their announcement arrives,
  with no allocation anywhere on the audio path.
- Memory grows to the busiest moment of the call and stays there: about
  120 KB per track (mostly the jitter buffer's 200 inline frames).
