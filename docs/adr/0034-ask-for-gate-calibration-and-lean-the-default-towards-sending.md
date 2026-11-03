# 0034. Ask for gate calibration, and lean the default towards sending

Status: Accepted · 2026-11-03

## Context

The voice gate opens on the first frame above its threshold, with no
lookahead (spec section 6), so a soft onset under the threshold is never
sent. The spec answers this with a calibration per user (section 10),
but nothing made anyone run it: a new microphone started at the core's
default, −40 dBFS.

The first test on real hardware, a wireless gaming headset on Windows,
showed how far off that can be. At −40 the start of words was cut ("Mais"
heard as "-ais"). The calibration wizard proposed −58 dBFS for the same
microphone, which kept their controller's clicks out; at −65 every word
came through whole. Microphones differ by tens of decibels; no default
suits them all.

Of the two ways a default can be wrong, a cut word costs the conversation,
while background noise sent through an open gate costs bandwidth and
lights the speaking indicator.

## Decision

- The core's default gate opens at −50 dBFS and closes at −56, ten
  decibels lower than before. It is one measurement, not a survey: the
  default leans towards sending until more microphones are tried.
- The desktop app stores the threshold only once the person has chosen
  it, by calibrating or by moving the slider (`gateOpenDbfs` is null
  until then). While it is unset, the home view shows a banner that asks
  for the ten-second calibration and leads to it, and the call panel says
  under the slider that the threshold is the default.
- Settings files from before keep the threshold they hold: a number there
  counts as chosen.

## Consequences

- The command-line client and every app inherit the new default.
- A microphone louder than the default expects sends its background
  noise until it is calibrated. The banner says so.
- The calibration itself is unchanged: it was right on the one microphone
  it was tried with.
