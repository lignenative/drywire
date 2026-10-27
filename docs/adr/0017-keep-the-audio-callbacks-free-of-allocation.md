# 0017. Keep the audio callbacks free of allocation

Status: Accepted · 2026-10-27

## Context

The audio callbacks run on the OS's real-time threads, with a deadline of
one I/O buffer: a fraction of a millisecond on a Mac, a few milliseconds on
Windows. Missing it is a click. Allocating or freeing memory can miss it:
the allocator takes locks, and may ask the OS for pages, so its worst case
is unbounded even when its average is fast. ADR 0014 accepted a few
allocations per frame for phase 1; this slice removes them before the
first physical tests, so that a glitch heard there is not this.

## Decision

The capture and playback callbacks neither allocate nor free, once the
call has started.

- **Sending.** The `Sender` encodes, frames and seals into buffers made
  once, with room for the largest frame and packet (`Packets`,
  `Encoder::encode_into`, `AudioPayload::encode_into`,
  `AudioKey::seal_into`). AES-GCM encrypts in place, and the message an
  announcement signs is a fixed-size array.
- **Frames are stored inline.** A received Opus frame travels as a
  `FrameBytes`, an array of 575 bytes and a length, from the network
  thread to the playback callback, through the jitter buffer, to the
  decoder. A `Vec` would be freed in the callback once played. 575 bytes
  is the largest frame the protocol allows (section 7): two of them fit in
  one datagram.
- **A wait-free ring** carries decrypted frames from the network thread to
  playback: `rtrb` (MIT or Apache 2.0, no dependencies), 256 slots made
  when the call starts. The standard channel allocates a block every 31
  messages and frees it on the receiving side. When the ring is full,
  because playback stalled, the network thread drops the frame and counts
  it (`playback_queue_full`).
- **The jitter buffer** keeps its frames in a sorted queue made with room
  for all of them, sorts the recent jitter in a scratch buffer of its own,
  and resets in place. A frame-size change reconfigures it in place too.
- **Tracks are made before playback starts**, one per member of the room
  (`Mixer::add_track`): making one creates an Opus decoder. A sender
  without a track still gets one on its first frame, which allocates;
  only tests and tools rely on that.
- **Statistics** are copied into a list that has room for every track.
- **A test enforces the rule.** `drywire-core/tests/realtime.rs` replaces
  the global allocator with one that counts calls made inside a callback,
  runs two calls through the real relay, and fails on the first
  allocation or free.

## Consequences

- Each sender keeps about 4 KB of buffers; each track about 120 KB (200
  frames of 600 bytes); the ring about 300 KB per call.
- A sender's frames over 575 bytes are dropped as `bad_framing`. Drywire's
  own encoder never produces one.
- The rule only covers Rust code, which is all the test sees. libopus does
  not allocate when encoding or decoding.
- The callbacks still take the statistics lock with `try_lock`, which never
  waits.
