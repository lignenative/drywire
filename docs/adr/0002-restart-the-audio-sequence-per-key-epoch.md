# 0002. Restart the audio sequence at each key epoch

Status: Accepted · 2026-10-27

## Context

The AES-GCM nonce of an audio packet is its 32-bit sequence number. A nonce
must never repeat under one key, so the sequence must never repeat within one
key epoch. The specification left open what happens to the sequence when the
sender changes epoch, and which packets advance it.

## Decision

- Only audio packets carry the audio sequence. It does not advance while the
  voice gate is closed, so a gap in sequence numbers always means a lost
  packet. Control packets use the field for their own purpose (ping number) or
  set it to zero.
- Each key epoch starts at sequence 0. A sender must move to a new epoch
  before its sequence reaches 2³¹.
- Replay windows, on receivers and on the relay, are kept per sender and per
  epoch.

## Consequences

- The relay can count uplink loss from sequence gaps without decrypting
  anything.
- Within an epoch, a sequence number never wraps, so comparisons stay plain
  integer comparisons, with no modular arithmetic.
- When a sender changes epoch, the relay and the receivers start a fresh
  window. A few packets from the old epoch that arrive late are still
  forwarded and decrypted (receivers keep the previous key), but the relay
  does not count them.
