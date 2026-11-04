# 0047. End the oldest event stream rather than refuse a new one

Status: Accepted · 2026-11-04

## Context

An identity may hold 8 event streams at once (ADR 0021), and until now
the server refused a ninth with `429`. The server frees a stream when its
client closes the connection, or when writing to it fails.

When a client's network changes (another Wi-Fi, a laptop waking up with a
new address), its old connections die without a word: no packet ever
tells the server, and writing a keepalive into a dead connection
succeeds until TCP gives up on it, which can take minutes. The client,
meanwhile, notices the silence after 45 seconds (protocol section 13.6)
and opens new streams. Since the desktop app follows up to 6 spaces per
instance besides its call's own space (ADR 0045), 7 new streams met
7 dead ones still counted, and some were refused: possibly the call's
own, so joining a room failed, and a reconnection kept retrying, until
the server noticed the dead connections.

## Decision

- A ninth stream for one identity ends that identity's oldest stream, by
  order of opening, instead of being refused. It is the rule a fifth
  login already follows for tokens (section 13.2).
- The limit stays at 8, and stays a bound on memory: one identity still
  cannot hold more.
- A client must not open more than 8 streams itself, or each new one
  would end one of its own live ones. The desktop app follows at most 6
  spaces per instance for this reason.

## Consequences

- After a network change, new streams are served at once; the dead ones
  are ended by the newcomers rather than by TCP.
- A stream can now end because the same identity opened newer ones. The
  client sees an ordinary end of stream and opens a new one, which it
  already does for every other end, so earlier clients need no change.
  A client facing an earlier server still gets `429` in that case and
  retries, as before.
- `presence.ErrTooManySubscriptions` and the `429` answer of the events
  endpoint are gone.
