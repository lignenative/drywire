# Measurements

Every latency figure in Drywire's documentation comes from this file,
with the setup that produced it (spec section 11). Physical mouth-to-ear
measurements, the ones that count, use the method of the spec; until we
have them, the software path is measured on its own and labelled so.

## 2026-10-27 — software path over loopback, no sound card

What it covers: framing, Opus, encryption, the relay, the jitter buffer
and decoding, twice (there and back). Not covered: the sound card, the
operating system's audio stack, a real network.

- Setup: MacBook Pro (Apple silicon), macOS, one machine. Relay
  `drywire-server` dev build on `127.0.0.1:10050`. Two `drywire-cli`
  release builds in file mode (`make live` with
  `ARGS="--input click.wav --output echo.wav --frame F"`): alex sends
  50 ms tone bursts at frame size F, sam is the echo bot at 5 ms frames.
  The file device runs a 2.5 ms period. Software at commit "Add
  drywire-cli".
- Measured: delay between the start of each burst in alex's input and the
  start of its echo in alex's output, five bursts.

| Alex's frames | Sam's frames | Echo delay (both legs) |
| --- | --- | --- |
| 2.5 ms | 5 ms | 19.7 to 20.0 ms |
| 5 ms | 5 ms | 25.0 ms |
| 10 ms | 5 ms | 37.5 ms |

At 5 ms frames each leg takes 12.5 ms: 5 ms to fill a frame, 2.5 ms of
Opus lookahead, and a one-frame jitter buffer, as the budget of spec
section 4 predicts. Loopback adds no measurable network delay (ping
round trip 0.1 to 0.3 ms).

## 2026-10-27 — software path again, before and after the hardening slice

The same path as above, measured again on the same machine to compare
the code before the slice that hardened the audio path (commit 5b37d17)
with the code after it (commit "Start a talk spurt within half a
millisecond of its schedule"). Both were built and run back to back.

- Setup: as above, with ten 50 ms bursts of a 1 kHz tone, 600 ms apart,
  and `--duration 8`. Each line is three runs of ten bursts. The first
  burst is shown apart: it is each sender's first talk spurt, which the
  jitter buffer schedules from a single packet.
- Measured: delay between the start of each burst in alex's input and
  the start of its echo in alex's output (first sample above 0.05).

| Alex's frames | First burst, before | First burst, after | Bursts 2 to 10, before | Bursts 2 to 10, after |
| --- | --- | --- | --- | --- |
| 2.5 ms | 20.0 to 22.5 ms | 20.0 to 22.5 ms | 17.5 to 20.0 ms (mostly 20.0) | 16.1 to 17.5 ms |
| 2.5 ms, redundancy | 22.5 to 25.0 ms | 21.0 ms | 20.0 to 22.5 ms | 18.0 to 20.0 ms (mostly 19.0) |
| 5 ms | 25.0 to 30.0 ms | 25.0 to 27.5 ms | 25.0 to 27.5 ms | 21.5 to 23.0 ms |
| 10 ms | 35.0 to 37.5 ms | 35.0 to 37.5 ms | 35.0 to 37.5 ms | 31.0 to 33.0 ms |

What changed it: a spurt used to start on the first playback tick of its
track after its scheduled time, and an idle track ticked once per frame,
so each leg waited up to a frame more (half a frame on average). Idle
tracks now tick every 0.5 ms. The first burst is unchanged although the
jitter buffer now gives a sender's first spurt two frames until its
second packet shows whether it uses redundancy; with redundancy it now
gets those two frames from the start, as it should, and is still faster
than before.

The steady values move by up to 2.5 ms from run to run: the file device
and the echo bot pace their 2.5 and 5 ms periods with sleeps, and where
their ticks fall relative to each other changes from run to run.

## 2026-10-27 — what Core Audio reports on the development Mac

Reported by the OS through `drywire-cli devices`, not measured. Built-in
devices of a MacBook Pro (Apple silicon), 48 kHz.

| Device | Smallest buffer | Reported latency beyond the buffer |
| --- | --- | --- |
| Built-in microphone | 15 frames (0.31 ms) | 1489 frames, 31.0 ms (device 14, safety offset 36, stream 1439) |
| Built-in speakers | 15 frames (0.31 ms) | 798 frames, 16.6 ms (device 60, safety offset 48, stream 690) |

`devices --play-silence` confirmed the output runs with 15-frame
callbacks (3202 per second). The built-in microphone's stream latency
alone is 30 ms, which suggests a wired headset or USB microphone for
low-latency use on a Mac. To be checked with the physical method.
