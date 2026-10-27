# drywire-cli

The command-line client. It creates spaces and invitation links on a
Drywire instance, joins a room, captures, sends, receives, mixes and plays,
and prints every second what the network and the jitter buffers are doing.
It is how we measure Drywire before any app exists (spec section 14).

## Try it on one Mac

```sh
make live
```

builds the server and the CLI, starts a server on `127.0.0.1:10050`, makes
a space, and invites an echo bot ("sam") that sends back everything it
hears; then you join as "alex" with your default microphone and speakers.
Speak and you hear yourself come back through the relay twice. **Use
headphones**: with speakers, the echo reaches the microphone again and
loops. macOS asks once for microphone access for your terminal. Options go
through `ARGS`:

```sh
make live ARGS="--frame 2.5"
```

## A team on a real instance

The owner of the space, once (their key must be in the instance's
`DRYWIRE_SPACE_CREATORS`, see `drywire-cli identity`):

```sh
drywire-cli create-space --instance https://voice.example.org --name Team --display-name alex
drywire-cli invite --uses 5            # prints https://drywire.gg/i#…
```

Everyone else, with the link they were sent privately:

```sh
drywire-cli join --link 'https://drywire.gg/i#…' --display-name sam
```

From then on, `drywire-cli join` is enough (`--space` and `--room` choose
when there are several). People who join or leave the room are added to or
removed from the call as it runs.

## Commands

```sh
drywire-cli identity                 # this device's public key
drywire-cli create-space --instance URL --name NAME
drywire-cli invite [--space NAME] [--uses N] [--days D]
drywire-cli accept LINK              # become a member without joining yet
drywire-cli spaces                   # the spaces this device knows
drywire-cli join [--space NAME] [--room NAME] [--link LINK] [options]
drywire-cli rename NAME              # your display name
drywire-cli export                   # what the instances store about you
drywire-cli leave-space [--space NAME]
drywire-cli delete-identity          # delete yourself from the instances
drywire-cli devices [--play-silence] # what the sound card gives
drywire-cli help                     # every option
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
for scripts. If the server ends the session (the same identity joined
elsewhere, the member was removed, or the relay heard nothing for 30
seconds), `join` stops and says so.

## What stays on this device

Everything lives in one directory: `--home DIR`, else `DRYWIRE_HOME`, else
`~/.drywire` (`%APPDATA%\drywire` on Windows).

- `identity.json`: the device's Ed25519 key, made on first use. The apps
  will keep it in the system's secure storage (spec section 8); the CLI
  keeps it in a file only you can read (mode 0600 on macOS and Linux; on
  Windows, your profile's permissions).
- `spaces.json`: for each space, the instance, its id and name, and the
  space key from the invitation link. The server never sees that key.

Both hold secrets: whoever copies them can speak as you and hear your
spaces. `delete-identity` removes you from the instances and forgets the
key.

## Files instead of a sound card

`--input in.wav` captures from a 48 kHz WAV file and `--output out.wav`
records what would have played; `--echo` sends back what it receives. Any
of them replaces the sound card entirely, paced in real time, so a call
can run headless and **never opens the microphone**. The tests use this:
`tests/two_clients.rs` makes a space with one CLI, joins it with the
invitation link from another, and checks what crossed the real server.

## Build and run on Windows

The Windows backend uses WASAPI: exclusive-mode capture when the
microphone is not used by another app, shared-mode playback with the
smallest period `IAudioClient3` offers, so game sound keeps playing on the
same headset. Playback keeps only one period queued (ADR 0015).

1. Install Rust with `rustup` (the MSVC toolchain, the default), the
   "Desktop development with C++" workload of Visual Studio Build Tools,
   and CMake (`winget install Kitware.CMake`). Building libopus needs the
   last two.
2. In a "Developer PowerShell for VS":

   ```powershell
   cd core
   cargo build --release -p drywire-cli
   .\target\release\drywire-cli.exe devices
   .\target\release\drywire-cli.exe join --link "https://drywire.gg/i#..." --display-name sam
   ```

3. Windows may ask to allow the program through the firewall: it only
   needs outgoing HTTPS to the instance and UDP to the relay.

`devices` shows the buffers and latencies Windows gives: it initializes
each stream as `join` would and releases it without starting it, so for an
instant it holds the microphone in exclusive mode. To use the microphone in exclusive mode, allow it in the sound
settings of the device ("Allow applications to take exclusive control");
if another app holds the microphone, the CLI falls back to shared mode and
says so on start.
