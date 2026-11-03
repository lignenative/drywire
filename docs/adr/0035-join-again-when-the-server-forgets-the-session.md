# 0035. Join again when the server forgets the session

Status: Accepted · 2026-11-04

## Context

The spec asks for "transparent reconnection" (section 10). Two layers can
lose a call:

- The relay binding. The core's link already handles it: without a pong
  for five seconds it says `hello` again, and a short network drop heals
  without the app doing anything.
- The session itself. The relay forgets a session it has not heard from
  for 30 seconds, and the server keeps sessions in memory only, so a
  restart forgets them all. The app learns it from its event stream: a
  `disconnect` for its own session, or, once the stream comes back, a
  snapshot without it. Until now the call then ended, and the person had
  to press "Join again".

Not every end of a session is a loss. A `leave` for this device's own
session means it joined from elsewhere or was removed: joining again on
its own would fight that.

## Decision

- A `disconnect` for this device's session, or a snapshot without it,
  is a lost session. The desktop app's session thread stops the call and
  the sound card, keeps the people list on screen marked as reconnecting,
  and joins the same room again: after 1 second, then 2, 4, 8, and every
  15 seconds after that, until it succeeds or the person leaves.
- Only temporary failures are retried: the server or the relay could not
  be reached, or the server answered 5xx or 429 (a new error code,
  `unavailable`). Any other refusal (the room is gone, the device is no
  longer a member) ends the call with its reason.
- A `leave` for this device's session still ends the call.
- Mute, the gate and every volume, including those changed while
  reconnecting, carry over to the new call. Nobody's arrival is announced
  again with a sound.

## Consequences

- A server restart costs each client a few seconds of silence, not a
  manual rejoin: the desktop test restarts the real server mid-call and
  checks that both apps hear each other again with their settings.
- The command-line client does not reconnect: it is a test and
  measurement tool, and a run that silently joins again would hide what
  it measures.
