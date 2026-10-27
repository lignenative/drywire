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
  `AudioKey::seal_into`). AES-GCM encrypts in place.

## Consequences

- Each sender keeps about 4 KB of buffers.
- The rule only covers Rust code. libopus does not allocate when encoding
  or decoding.
