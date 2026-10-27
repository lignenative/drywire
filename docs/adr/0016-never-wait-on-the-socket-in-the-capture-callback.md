# 0016. Never wait on the socket in the capture callback

Status: Accepted · 2026-10-27 · Amends ADR 0014

## Context

ADR 0014 sends packets from the capture callback, to save a thread hop. It
used a blocking socket, on the grounds that a UDP send only waits when the
socket's send buffer is full, which a few hundred small packets per second
do not fill. That is true on an idle link, but not a guarantee: on Linux a
UDP send blocks while the kernel still holds a socket buffer's worth of
datagrams (a congested Wi-Fi link, a slow interface), and a blocked capture
callback misses its deadline. The OS then drops microphone samples, which
is a glitch on every frame until the link recovers.

Two ways out:

- **Hand packets to a network thread** through a queue. The callback never
  touches the socket, but every packet waits for that thread to wake up:
  tens of microseconds on an idle machine, milliseconds on a busy one.
- **Make the socket non-blocking.** A send either completes at once or
  fails with `WouldBlock`. A packet refused that way is lost, and the
  receiver conceals it as it would a packet lost on the network. No hop,
  no added latency.

The catch is that the network thread used the same socket's read timeout
to sleep until a packet arrived, and a read timeout only works on a
blocking socket (the blocking mode belongs to the socket, not to one
thread).

## Decision

- The call's socket is **non-blocking** once the session is bound.
- The capture callback sends at once; on `WouldBlock` it **drops the packet
  and counts it** (`send_dropped`, shown by `drywire-cli`). Other errors
  are counted apart (`send_errors`).
- The network thread waits for a datagram with the OS's own wait, `poll`
  on macOS and Linux and `WSAPoll` on Windows (`drywire-core/src/socket.rs`),
  then reads everything that arrived before waiting again. It wakes up as
  fast as a blocking `recv` did.

## Consequences

- No call on the capture path can wait on the network.
- A full send buffer shows as `send_dropped` and as concealment at the
  receivers, instead of as late frames and missed microphone deadlines.
- `drywire-core` depends on `libc` on Unix (MIT or Apache 2.0, already in
  the build through `getrandom`) and on one more feature of the `windows`
  crate.
