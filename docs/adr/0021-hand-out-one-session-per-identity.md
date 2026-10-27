# 0021. Hand out one session per identity, and end it when the relay stops hearing it

Status: Accepted · 2026-10-27

## Context

In phase 1 a session number and secret came from a file, and lasted as long
as the relay ran. In phase 2 the control plane hands them out when a member
joins a room (spec section 7), and the members of a space need to know who
is in which room, and who just lost their connection (spec section 10).

## Decision

- Joining a room creates a **fresh session**: a random 32-bit number and a
  random 32-byte secret, added to the relay at once. Leaving, being
  removed, or the room or space being deleted takes it off the relay.
- An identity holds **one session** on the instance. Joining another room
  ends the previous one, so a device is in one room at a time and the
  relay never forwards to a session nobody uses.
- **Disconnect** means the relay has not heard from the session for its
  timeout (30 seconds, three missed keepalives). The relay removes it and
  the control plane announces a `disconnect`. A session that never said
  `hello` expires the same way. A client whose session expired joins the
  room again; a session is cheap. A shorter timeout would misread a client
  whose gate is closed and pings are off, which sends one keepalive every
  10 seconds.
- Presence is **memory only** (spec section 9): no history, nothing
  stored. Server-Sent Events carry it, from the standard library: a
  snapshot first, then one event per change. A subscriber that falls 256
  events behind is dropped rather than waited for; it reconnects and gets a
  new snapshot. Each identity may hold 8 streams.
- The relay and the presence hub each have their own lock, and neither
  calls the other while holding it. The control plane asks the relay every
  second which sessions expired (`Expire`), instead of the relay calling
  out from its packet loop.

## Consequences

- Clients learn the public keys of new members from the event stream, so
  they can verify their announcements before the first audio packet.
- A member removed while joining could keep a session for a moment; the
  join checks membership again once its session is visible, and ends it if
  the member is gone.
- A server restart ends every session and stream; clients log in and join
  again.
