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
