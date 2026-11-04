# 0041. Close the lock only when everyone heard is verified

Status: Accepted · 2026-11-04

## Context

The apps show a discreet lock during a call, to prove that the audio is
end-to-end encrypted; the project's site will say so too, second to
latency. A lock that shows closed while nothing was checked would be a
promise, not a proof, so its states need a precise definition that the
code computes from what it really checked.

What a receiver checks today (protocol section 8):

- each sender's **announcement**, an Ed25519 signature over the salt their
  key derives from, verified with the public key the control plane gave
  for their session; a bad one is dropped (`bad_signature`);
- each **audio packet**, authenticated by AES-GCM under that key; a bad
  one is dropped (`bad_tag`).

Audio that arrives before its sender's announcement cannot be decrypted
and is dropped too (`no_key`); the next announcement is at most a second
away. A member who stays silent sends nothing after their first
announcement, which a receiver who joined later never saw.

## Decision

`Inbound` keeps, per member of the room, whether an announcement of
theirs verified, whether audio of theirs arrived, and their own
`bad_signature` and `bad_tag` counts (`SenderCheck`). From those,
`drywire-core`'s `encryption` module gives each sender a state:

- **verified**: an announcement of theirs verified with their public key,
  and nothing in their name failed in the last 10 seconds;
- **waiting**: their audio arrives, but no announcement of theirs verified
  yet;
- **silent**: nothing heard from them since this device joined;
- **failed**: in the last 10 seconds, an announcement in their name failed
  its signature, or an audio packet in their name failed authentication.

And the room one, which is what the lock shows:

- **failed** if any sender is failed;
- **verified** if no sender is waiting and at least one is verified;
- **pending** otherwise: alone, everyone silent so far, or someone heard
  whose announcement has not arrived.

- **Silent members do not hold the lock open.** Nothing of theirs is
  played, so nothing of theirs needs proving; waiting for them would keep
  the lock pending in every room with a quiet player.
- **Nobody verified means pending**, never verified: a lock with nothing
  behind it would be the promise this ADR avoids.
- **A failure shows for 10 seconds, then clears** once that sender's
  packets verify again. The failed packets were dropped, never played, so
  what plays afterwards is still proven. A failure that never cleared would
  leave a call alarming for good over one packet, and a sender who
  restarts can make a few in-flight packets fail honestly (section 8.3).
  The counters keep the whole call's failures for the statistics.
- **A failure counts for the session named in the packet's header**,
  whoever really sent it: the receiver cannot tell, and either way someone
  tried to inject packets in that member's name.
- The call's **network thread** derives the state when it publishes its
  statistics, every 100 ms, and it reaches the app in `CallStats`. The
  audio callbacks do no work for it (ADR 0017, 0032).
- Neither the state nor the counters hold anything secret: public keys,
  sessions and counts. Nothing about it is logged.

## Consequences

- What a closed lock proves, and what it does not, is said the same way
  everywhere (app, site, documentation):
  - every voice played came from a key whose owner signed it with the
    identity the server listed for them, and every frame was
    authenticated; the relay forwards packets it cannot read or forge,
    because the space key travels only in the invitation link's fragment
    and never reaches the server;
  - it does not prove who a public key belongs to: the server lists them,
    and nothing yet lets two people compare their keys out of band;
  - any member of the space holds the room key and could forge frames in
    another member's name (spec section 8, known limit): the lock protects
    against the server and the network, not against a member.
- A newcomer who talks is verified within their first words, since every
  talk spurt starts with an announcement. Joining a room where someone is
  mid-sentence shows pending for at most a second.
- Alone in a room, the lock is pending. The interface decides how to show
  that; it must not show it closed.
- `tests/realtime.rs` checks the state through a real relay; the
  `encryption` module's tests cover each state, a forged announcement and
  a tampered frame.
