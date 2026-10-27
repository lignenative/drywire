# 0007. Send sender announcements through the relay

Status: Accepted · 2026-10-27

## Context

A receiver can only decrypt a sender's audio once it knows that sender's
`sender_salt` for the current key epoch (protocol section 8). The protocol
said the salt is published, signed, "through the control plane". Phase 1 has
no control plane, and even in phase 2 a separate HTTPS round trip for every
key epoch would delay the first words of a new member.

Options considered:

- **A new UDP packet type that the relay forwards like audio.** The salt
  travels on the path the audio already takes; nothing else is needed.
- **Put the salt in a configuration file.** Breaks the point of the salt: it
  must be fresh randomness for every epoch and every restart, which a static
  file cannot provide.
- **Carry the salt in every audio packet.** 32 bytes plus a 64-byte
  signature on each 2.5 ms frame would double the packet size.
- **Wait for the control plane.** Blocks phase 1 for no gain.

## Decision

- A new packet type, `announce` (`0x07`), carries the announcement:
  `header ‖ sender_salt (32) ‖ signature (64)`, exactly 112 bytes. The header
  says which session and key epoch it is for; the signed message is the one
  section 8.3 already defined, so no new cryptography is introduced.
- The relay treats it like audio for routing and rate limiting: it accepts it
  only from the bound address of the session, and copies it unchanged to the
  other bound members of the room. It does not count it as audio and does not
  try to verify the signature, which it could not do without the members'
  public keys.
- UDP may lose it, so the sender repeats it: when the epoch starts, before
  the first audio packet of every talk spurt, and then once per second while
  its gate stays open. At 112 bytes per second this costs nothing next to the
  audio, and a member who joins mid-sentence can decrypt within a second.
- Receivers verify the signature with the public key they already know for
  that session (phase 1: the client configuration file, ADR 0006; phase 2:
  the control plane). An announcement for an unknown session, or with a bad
  signature, is dropped (`bad_signature`).
- Audio that arrives before its announcement cannot be decrypted. Receivers
  drop it and count it (`no_key`); they do not hold it back, because the next
  announcement is at most one second away and holding audio would add
  latency.
- Adding a packet type is not a breaking change (protocol section 11), so the
  protocol version stays 1.

## Consequences

- Phase 1 needs no control plane for encryption, and phase 2 needs no new
  mechanism: the control plane only distributes public keys.
- An attacker who can replay an old announcement (the relay itself, for
  example) can make a receiver switch to an old salt for one epoch. That only
  makes audio undecryptable, which the same attacker could already achieve by
  dropping packets. It never weakens confidentiality: receivers do not
  encrypt with a sender's key.
- The first words of a talk spurt are protected against a lost announcement
  only when the receiver already has the epoch's key, which is the normal
  case after the first second in a room.
