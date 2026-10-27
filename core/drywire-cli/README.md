# drywire-cli

The phase 1 client: it joins a room on a relay, captures, sends, receives,
mixes and plays, and prints every second what the network and the jitter
buffers are doing. It is how we measure Drywire before any app exists
(spec section 14).

## Try it on one Mac

```sh
make live
```

builds the relay and the CLI, starts a relay on `127.0.0.1:10050`, an echo
bot ("sam") that sends back everything it hears, and joins as "alex" with
your default microphone and speakers. Speak and you hear yourself come back
through the relay twice. **Use headphones**: with speakers, the echo
reaches the microphone again and loops. macOS asks once for microphone
access for your terminal. Options go through `ARGS`:

```sh
make live ARGS="--frame 2.5"
```

## Commands

```sh
drywire-cli join --config FILE [options]   # talk in a room
drywire-cli devices [--play-silence]       # what the sound card gives
drywire-cli new-room --relay HOST:PORT --members alex,sam,kim --out DIR
drywire-cli help                           # every option
```

`join` prints a line per second:

```text
[   2s] rtt 0.22 ms (min 0.10 ms) | up 409 sent, 0 lost, talking | sam: buffer 2 (target 5.00 ms), played 401, concealed 0, late 0, dropped 0
```

- `rtt`: round trip to the relay, from the pings.
- `up … lost`: audio packets the relay expected from us but never received.
- per sender: frames waiting in its jitter buffer, the buffer's target
  delay, and how many frames were played, concealed (lost or late: Opus
  filled the gap), arrived too late, or were dropped to catch up.

When it stops it prints a `summary key=value …` line on standard output,
for scripts.

## Files instead of a sound card

`--input in.wav` captures from a 48 kHz WAV file and `--output out.wav`
records what would have played; `--echo` sends back what it receives. Any
of them replaces the sound card entirely, paced in real time, so a call
can run headless and **never opens the microphone**. The tests use this:
`tests/two_clients.rs` runs two CLIs through a real relay with WAV files.

## Configuration files

A client reads everything it would learn from the control plane from one
JSON file (ADR 0006; the format is documented in
`drywire-core/src/config.rs`). `dev/` holds the public development files
for the room "lobby" of `server/rooms.dev.json`; they are marked
`dev_only` and refuse any relay that is not on this machine. They were
written with:

```sh
drywire-cli new-room --dev --relay 127.0.0.1:10050 --members alex,sam,kim --room lobby --out dev
```

For a real test over the internet, generate fresh keys:

```sh
drywire-cli new-room --relay relay.example.com:10050 --members alex,sam,kim --out team
```

This writes `team/rooms.json` for the relay (`DRYWIRE_ROOMS`) and one file
per member. They hold secrets: give each person only their own file,
privately.

## Build and run on Windows

The Windows backend uses WASAPI: exclusive-mode capture when the
microphone is not used by another app, shared-mode playback with the
smallest period `IAudioClient3` offers, so game sound keeps playing on the
same headset.

1. Install Rust with `rustup` (the MSVC toolchain, the default), the
   "Desktop development with C++" workload of Visual Studio Build Tools,
   and CMake (`winget install Kitware.CMake`). Building libopus needs the
   last two.
2. In a "Developer PowerShell for VS":

   ```powershell
   cd core
   cargo build --release -p drywire-cli
   .\target\release\drywire-cli.exe devices
   .\target\release\drywire-cli.exe join --config team\alex.json
   ```

3. Windows may ask to allow the program through the firewall: it only
   needs outgoing UDP to the relay.

`devices` shows the buffers and latencies Windows gives, without starting
a stream. To use the microphone in exclusive mode, allow it in the sound
settings of the device ("Allow applications to take exclusive control");
if another app holds the microphone, the CLI falls back to shared mode and
says so on start.
