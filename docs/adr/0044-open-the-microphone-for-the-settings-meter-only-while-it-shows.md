# 0044. Open the microphone for the settings' meter only while it shows

Status: Accepted · 2026-11-04

## Context

The settings' Voice page draws the microphone's level under the voice
gate's threshold, so a person can set the threshold against their own
voice before joining anyone (the design canvas's V3 page). Until now the
desktop app read the microphone only in a call and during the latency
lab, so out of a call the meter stayed empty, and choosing a threshold
meant joining a room first.

Reading the level means opening the sound card. A microphone held open
shows the system's "in use" indicator and keeps the device from the call
and the lab, which need it alone.

## Decision

- A command, `input_level`, gives the level in dBFS. In a call it is the
  call's. Out of a call the first request opens the default sound card
  on a thread of its own (`desktop/src-tauri/src/meter.rs`), measuring
  the loudest 5 ms frame between two reads (`drywire_core::lab::LevelMeter`),
  with the call meter's fall of 50 dB a second.
- The interface asks ten times a second while the meter is on screen and
  the page is visible. The Rust side closes the sound card by itself one
  second after the last request: leaving the page or a frozen interface
  lets the microphone go, with no command to forget. A request while the
  main window is hidden in the tray or minimized closes the meter and
  reads the floor, so a web view that keeps its timers running while
  hidden cannot hold the microphone either.
- Joining a room, the loopback test and the calibration close the meter
  first, and wait until its sound card is closed. While one of them holds
  the card, the meter reads the floor (-90 dBFS) instead of waiting.
- Nothing measured leaves the meter's thread except the one level the
  interface asks for: no recording, no audio kept.

## Consequences

- The threshold can be set, and checked, out of a call: the meter turns
  green above it as in a call.
- The microphone indicator shows while the Voice page is open, which is
  what the person is doing there. It goes out a second after they leave.
- A missing or refused microphone makes the command fail once; the
  interface then stops asking and leaves the meter empty.
- The microphone indicator going out when the window is hidden is to be
  checked on Windows (desktop/README.md, "Testing on Windows").
