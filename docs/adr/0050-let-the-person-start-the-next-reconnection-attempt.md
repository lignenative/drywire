# 0050. Let the person start the next reconnection attempt

Status: Accepted · 2026-11-04

## Context

ADR 0035 joins a lost call again on its own, after 1 second, then 2, 4,
8, and every 15 seconds after that. The pauses grow so that a server
that is down, or a network that is gone, is not asked again every
second by every client.

Trying the web client on a laptop, the owner cut the Wi-Fi and turned it
back on. The page came back into the room by itself, but only at the
next attempt: up to 15 seconds after the network was back, with the
banner counting down and nothing to do but wait. The person often knows
before the app does that the network is back: they just reconnected it.

## Decision

- While a lost call waits for its next attempt, the banner that says so
  offers "Try now" ("Réessayer maintenant"). A click starts the waiting
  attempt at once, in the desktop app (a `retry_now` command to the
  session thread) and in the web client alike, through the interface's
  `Api` (`retryNow`).
- The button shows only while an attempt is waiting, that is while the
  countdown shows. During an attempt there is neither countdown nor
  button, and a click that still reaches the app then does nothing: the
  attempt running is what it asked for, so a call never joins twice. The
  session thread drops such a click once the attempt ends; the web
  client has no wait to cut short at that moment.
- An attempt the person starts restarts the sequence: if it fails, the
  next one comes 1 second later, then 2, 4, 8 and 15. The person clicked
  because the network should be back; if it is still coming up (an
  address being assigned, a name not resolving yet), a second later is
  when it most likely works, not 15 seconds later. Each click costs the
  server one request, at the pace of a person clicking, which the pauses
  of ADR 0035 were never meant to limit.
- Everything else ADR 0035 decided holds: which failures are retried,
  what carries over to the new call, and that leaving never waits for an
  attempt.

## Consequences

- The person can cut a reconnection short after fixing their network;
  the test of the desktop app restarts the server while the pauses have
  reached 8 seconds and checks that a click joins at once, and that a
  click that fails brings the next attempt a second later.
- A person who keeps clicking while the server is down sends one join
  per click. That is a person's pace, and every one of them fails fast.
- The web client's schedule lives in `apps/web/src/retry.ts`, a copy of
  the session thread's `Retry` in TypeScript, tested on its own.
