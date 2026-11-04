# web

Drywire in a browser, for people who will not install an app first. It
will always be slower than a native client; it is the easy door, not
the way to play (spec section 10), and it says so in its settings.

It is the desktop app's own interface (`desktop/src/`, ADR 0048) on a
live call: the same React components, drawn from the design canvas,
driven by a third implementation of the app's commands (`src/api.ts`)
built on WebTransport, the protocol in WebAssembly, Opus in WebCodecs and
two AudioWorklets. What a browser cannot do (the overlay, a mute shortcut
that works in a game, launching at login, the loopback test) is left out;
what only a browser needs (an unsupported-browser page, the microphone's
permission) is drawn on the canvas's V3 page, row "Navigateur · client
web", and built in `src/components/`.

## Try it

```sh
rustup target add wasm32-unknown-unknown   # once
make web
```

`make web` builds the server, the command-line client and the
WebAssembly module, starts a server with its WebTransport side
(`127.0.0.1:10058/udp`), puts an echo bot named sam in a room, and serves
the web client with Vite on `127.0.0.1:10057`. Open the address it
prints: the invitation is in its fragment, which the browser never sends
anywhere, and the client offers to join the space with it. Then join the
room, allow the microphone, and hear yourself back from sam.

In development, `?scene=unsupported` shows the unsupported-browser page
in any browser, to compare it with the canvas. Compare the wide window at
1120 × 700 and the narrow one at 380 × 640, as for the desktop app.

It needs WebTransport, Opus in WebCodecs, AudioWorklet, WebAssembly and
Ed25519 in WebCrypto: Chrome or Edge 137+, Firefox 136+ on the desktop,
Safari 26.4+ (ADR 0038 to 0040). Only Chromium has been tried so far
(see "Measure").

## On an instance

Every instance deployed with [`deploy/`](../../deploy/README.md) serves
this client at its own address, beside its API: the reverse proxy sends
`/v1/…` to the control plane and the rest to `drywire-static`, which
serves this directory's build (ADR 0049). `make deploy` builds it. Page
and control plane share one origin, so no cross-origin request is made.
To open an invitation in a browser, keep everything from the `#` of the
link and put the instance's address before it:
`https://voice.example.org/#…`.

## How it is built

| File | What it does |
| --- | --- |
| `src/main.tsx` | Checks the browser, loads the module and the identity, hands the interface its commands, renders it |
| `src/api.ts` | The interface's commands (`Api` of `desktop/src/api.ts`) in a browser |
| `src/call.ts` | A call: join, bind to the relay, send, receive, presence, the lock, joining again (ADR 0035, 0041) |
| `src/audio/engine.ts` | The microphone and the speakers: the AudioContext, the two worklets, the permission |
| `src/audio/capture.worklet.ts` | On the audio thread: frames of the microphone, their level, the voice gate |
| `src/audio/playback.worklet.ts` | On the audio thread: one jitter buffer per person, volumes, the mix |
| `src/audio/gate.ts`, `src/audio/jitter.ts` | The gate, its calibration, the jitter buffer and the soft clip, after `drywire-core` |
| `src/codec.ts` | Opus through WebCodecs, on the main thread (ADR 0040) |
| `src/protocol.ts` | `drywire-wasm` ([`core/drywire-wasm/`](../../core/drywire-wasm/)): the protocol and the encryption (ADR 0040) |
| `src/identity.ts` | The device's Ed25519 key, non-extractable, in WebCrypto and IndexedDB |
| `src/control.ts` | Login, invitations, spaces, join, the presence stream (protocol section 13) |
| `src/relay.ts` | The WebTransport session, pinned by the certificate hashes the join answer gives (ADR 0038, 0039) |
| `src/store.ts` | The spaces joined, with their keys, and the preferences, in this site's storage |
| `src/components/` | The screens only the web client has: unsupported browser, microphone |

The interface's files are imported from `desktop/src/` through the `@/`
alias; every package they use resolves to this directory's
`node_modules`, which therefore lists the same packages at the same
versions as `desktop/package.json`.

In development the page reaches the control plane through Vite's proxy
(`/v1`): the control plane does not answer cross-origin requests yet, so
the client reaches an instance only through its own origin or that proxy.

### The audio path, and why

- **Untouched voice.** The microphone is asked for with
  `echoCancellation`, `noiseSuppression` and `autoGainControl` off and
  one channel. Nothing changes a sample; the gate only decides what is
  sent. Turning the three off also lets Chrome use the device's own
  buffer instead of forcing 10 ms ones.
- **48 kHz everywhere**, Opus's rate: nothing resamples when the device
  runs at 48 kHz.
- **5 ms Opus frames at 64 kb/s**, the native clients' settings
  (`drywire-core`, `sender.rs`), CELT only where the browser honours
  `lowdelay` (Chrome). A browser that refuses 5 ms frames gets 10 ms.
- **Output buffer: `latencyHint: 0`**, the smallest the browser allows:
  one render quantum, 128 samples (2.7 ms) in Chrome on a Mac, where
  `"interactive"` gave 256. Neither glitched in the measurement below; if
  a machine does at 0, `"interactive"` is the next step.
- **Capture**: the worklet cuts the input into 5 ms frames and decides
  on each one at once, with the core's gate (instant open, 150 ms hold,
  6 dB hysteresis). WebCodecs does not exist in a worklet, so each frame
  the gate lets through crosses to the main thread to be encoded, sealed
  and sent. The announcement goes before every spurt and once a second
  during one.
- **Playback**: each person's frames are decoded on the main thread and
  handed to the playback worklet, which holds one jitter buffer per
  person, as the core's: the target is the 95th percentile of recent
  jitter plus half a millisecond, never under one frame, and moves only
  between talk spurts; more than two frames above it and the oldest is
  dropped. WebCodecs cannot conceal a lost frame: a missing one is
  silence. Volumes apply there, then a plain sum and the core's soft
  clip.

## Measure

What the interface shows, as in the app: the round trip to the relay
(the connection chip, measured), the device latency (input plus output
as the browser reports them, labelled "reported"), uplink loss, and for
each person their buffer target, counters (received, played, concealed,
late, dropped, buffer depth) and an estimated mouth-to-ear, labelled as
an estimate.

In development, the console has more: `await drywire.callStatus()` gives
the whole call view, and `drywire.playbackStats()` the output's
underruns where the browser counts them (Chrome's
`AudioContext.playbackStats`).

### Measured so far

2026-11-04, `make web` on one MacBook Pro (Apple silicon), the Claude
app's built-in browser (Chromium 152), loopback network, 60 seconds with
the echo bot. The built-in browser refuses the microphone, so the page's
`getUserMedia` was replaced from the console by a synthetic one: a
440 Hz tone, 0.4 s on and 0.6 s off. Software at this README's commit.

| | `latencyHint: "interactive"` | `latencyHint: 0` |
| --- | --- | --- |
| Output buffer (`baseLatency`) | 5.3 ms | 2.7 ms |
| Output latency the browser reports | 24 ms | 24 ms |
| Underruns (`playbackStats`) | 0 | 0 |
| Frames sent | 6,695 | 6,692 |
| Frames received from sam, played | 8,558, 8,558 | 8,540, 8,540 |
| Concealed, late, dropped | 23, 0, 0 | 4, 0, 0 |
| Jitter buffer target | 5.8 ms | 5.0 ms |
| Round trip to the relay (median of 5) | 0.3 ms | 0.3 ms |

The mute, the gate (raised above the tone, nothing is sent; lowered,
frames flow again), the per-person volume, the calibration and the
encryption lock (verified with sam) were checked on the same call. These
are not mouth-to-ear figures: nothing here went through a microphone or
a speaker, and a synthetic stream has no capture latency.

### Mouth to ear, the reference method

The only latency figure Drywire quotes comes from the physical method of
spec section 11, recorded in [`docs/measurements.md`](../../docs/measurements.md).
For the web client:

1. Two machines side by side, A and B, in the same room of the same
   instance, each with its own headset or earphone. A runs the web
   client in the browser under test; B runs the desktop app (or, to
   measure browser to browser, the web client too). Calibrate both gates
   so a click opens them.
2. A third recorder, a phone, lies between A's microphone and B's
   earphone, close enough to hear both.
3. Make at least 20 sharp clicks (two pens, a clicker) next to A's
   microphone, a second apart. The phone records each click twice: the
   direct sound, then B playing it.
4. Run `drywire-bench measure recording.wav`: it pairs the clicks by
   cross-correlation and gives the median and the 95th percentile.
5. Swap A and B to measure the other direction: a browser's capture and
   its playback do not cost the same.
6. Write the result in `docs/measurements.md` with the browser and its
   version, the operating system, both devices, the connection, the frame
   size (5 ms unless the browser refused it), the latency hint and the
   commit. Measure the desktop app on the same machines the same day:
   what a browser adds is the difference.

`make web` on one machine is not this measurement: the echo comes back
through sam's own jitter buffer and a second leg, so the delay from a
click to its echo holds both directions and the bot.

## Not verified yet

- Safari 26.4+ and Firefox 136+: nothing has run there. Safari writes
  datagrams with `createWritable()` (handled), Firefox and Safari ignore
  `lowdelay` and encode with more lookahead.
- A real microphone and speaker in a browser, and therefore any
  mouth-to-ear figure.
- Joining again after a lost session (ADR 0035) is written as in the app
  but has not been exercised: restart the server mid-call to try it.
- Who sits in rooms you are not in is not shown yet, as in the app.
- Muting in the middle of a talk spurt sends no end mark: the others'
  buffers conceal for up to 200 ms before they stop, and the name stays
  lit that long.

## Test

```sh
bun test           # the WebAssembly module against the vectors, the gate, the jitter buffer
bun run typecheck  # this client and the desktop interface it imports
bun run build      # the bundle, worklets included
```

`make test` builds the module into `public/` and runs all three.
