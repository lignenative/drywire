# 0033. Play the presence sounds outside the call

Status: Accepted · 2026-10-28

## Context

The spec (section 10, "Presence") asks for three short sounds when
someone joins, leaves or loses their connection, "played locally and
mixed low", that "never delay teammates' audio". They could be mixed
into the call's playback by the core's mixer, or played by the app
through another output.

Mixing them in the core means a fourth kind of track in the playback
callback, sound data reaching it without allocating, and a change in
every client's core for what is an interface feature. Played elsewhere,
they go through a second stream to the same device, which the system
mixes.

## Decision

- The desktop app plays them in its interface, with Web Audio: a few
  synthesised notes per sound, so there is no sound file to license or
  ship. They go through the web view's own audio stream, never through
  the call's.
- The session thread reports arrivals and departures to the window as
  events (`presence`), not through polling, so the sounds play while the
  window is hidden in the tray, where its timers are slowed down. The
  overlay hears the same events for its pills.
- Their volume is a setting, from 0 (off) to 100 %, and even at 100 %
  they peak a quarter of full scale.

## Consequences

- The call's audio path is unchanged: the sounds cannot delay or
  interrupt a teammate's voice. With WASAPI exclusive capture and shared
  playback, the system mixes the two output streams; a human check on
  Windows confirms it (`desktop/README.md`).
- The sounds' own latency (Web Audio's output) does not matter: they say
  that someone came or went, not when to the millisecond.
- The mobile and Apple apps play them with their own system sound APIs;
  the recipe (notes and lengths) is in `desktop/src/sounds.ts`.
