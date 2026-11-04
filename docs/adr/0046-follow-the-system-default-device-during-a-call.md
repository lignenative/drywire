# 0046. Follow the system's default device during a call

Status: Proposed · 2026-11-04 (designed, not built: it needs a Windows PC
with two audio devices to prove)

## Context

A call opens the system's default microphone and output when it starts
(`default_devices`, ADR 0013) and keeps them until it ends. A player who
plugs in a headset mid-match, or picks another device in Windows' sound
settings, keeps talking into the old one until they leave and join
again. Unplugging the device in use is worse: WASAPI returns
`AUDCLNT_E_DEVICE_INVALIDATED`, the stream's thread ends, and the call
goes silent in that direction with only a line in the log.

The core's audio contract makes the fix belong to the backend. A call
hands its capture and playback callbacks to `AudioIo::start` once
(`Call::join` makes them once), and they never wait, allocate or free
(ADR 0017). Nothing above the backend can reopen a device without those
callbacks, and nothing on their path may block on COM.

## Decision (proposed)

- **The WASAPI backend follows the default device by itself**, keeping
  the callbacks it was given. Each stream's thread runs its loop as today,
  and leaves it, without ending, when told its device is no longer the
  default or when WASAPI says the device is gone
  (`AUDCLNT_E_DEVICE_INVALIDATED`). It then stops and releases its
  client, opens the new default as it opened the first (exclusive capture
  first, shared playback with the smallest period), and runs again with
  the same callback. With no device at all, it tries again every 500 ms,
  and the callbacks are simply not called meanwhile: the call's sender
  sees no input, the jitter buffers conceal, nothing else changes.
- **The check runs off the audio threads.** A third thread, started and
  stopped with the streams, asks `IMMDeviceEnumerator::GetDefaultAudioEndpoint`
  (console role, as today) every 500 ms and compares each endpoint's id
  with the one its stream opened (`IMMDevice::GetId`, published by the
  stream's thread when it opens). A difference sets that stream's
  `moved` atomic; the stream's thread reads it once per wake-up, which
  costs one relaxed load. Polling, rather than an `IMMNotificationClient`
  callback, keeps COM objects of our own and their threading rules out of
  the backend, at the cost of noticing a change up to half a second late,
  which a person switching devices does not notice.
- **What the system gave is reported again.** `AudioIo` gains
  `fn streams(&self) -> Option<Vec<StreamInfo>>`, `None` by default, which
  the WASAPI backend answers from what its threads last opened. The
  desktop's session thread reads it at each tick and updates the call
  view's `devices` and `deviceMs` (a loopback measurement is kept only
  while the devices are the ones it measured).
- **The switch is a glitch, never lasting latency.** A reopened playback
  stream starts with its queue of one period of silence, as at the start
  (ADR 0015); the jitter buffers keep their targets.
- Core Audio on macOS gets the same treatment later, through
  `kAudioHardwarePropertyDefaultInputDevice` and `...OutputDevice`
  listeners; Windows comes first.

## How it will be tested

- What CI can check: the backend still compiles for Windows
  (`make test`'s Windows check, CI's Windows build); the session thread's
  handling of new `streams()` is tested with an in-memory device that
  changes its `StreamInfo` mid-call (`desktop/src-tauri/tests/app.rs`).
- What only a person on a Windows PC can check, added to
  `desktop/README.md` when built: in a call, switch the default output
  and input in Windows' sound settings, then plug and unplug a USB
  headset; within a second the voice moves to the new device both ways,
  the connection popover names it, and the teammate hears no more than a
  short gap. Exclusive capture is checked to be taken again on the new
  microphone.

## Consequences

- Until it is built, a player changing devices must leave and join
  again, and an unplugged device in use leaves that direction silent.
- Once built, the backend owns a third thread per call and the reopen
  logic: more code in the least testable file of the core, which is why
  this waits for a test on real hardware rather than landing untested.
