# desktop

Drywire's desktop app, for Windows first (phase 3 of
[the spec](../docs/spec.md)), Linux later. It is a [Tauri v2](https://tauri.app)
window whose interface is written in React and TypeScript, around the same
Rust core as every other client.

```text
desktop/
  src/             The interface: React + TypeScript, built by Vite
  src-tauri/       The Rust side: a Cargo workspace of its own (ADR 0030)
    src/app.rs       What each command does, without the window
    src/session.rs   A call in progress, on a thread of its own
    src/shell.rs     The tray icon, the global shortcut and the overlay
    src/latency.rs   The per-person latency estimate
    src/store.rs     The identity and the space keys on disk
    src/vault.rs     DPAPI on Windows (ADR 0031)
    tests/app.rs     The whole app against the real server, headless
  assets/icon.svg  The icon's source, a placeholder until the visual identity
```

The icons in `src-tauri/icons/` are generated from `assets/icon.svg` with
`bunx --bun tauri icon assets/icon.svg -o src-tauri/icons` (then remove the
mobile ones it adds).

## What it does

- Shows this device's public key, which an operator lists to let the
  device create spaces.
- Accepts an invitation link, and lists the spaces it knows with their
  rooms.
- Joins a room on the default sound card, and leaves.
- In a call, the compact widget (spec section 10): the instance and the
  ping to the relay, the room, everyone in it with a speaking light, the
  microphone's level meter with the voice gate's threshold on it, a mute
  button, the gate's threshold, and each person's volume from 0 to 200 %,
  with their jitter buffer and recent loss. The gate and the volumes are
  remembered (`settings.json`, by public key for volumes); mute is not.
- Out of the window, for a player in a match: a tray icon (show, mute,
  quit; closing the window only hides it), a global mute shortcut
  (`Ctrl+Shift+M` by default, changed in the settings), three short
  presence sounds when someone joins, leaves or loses their connection
  (synthesised by the interface, played through the web view's own output,
  never through the call), and the overlay: a transparent, always-on-top
  window in the top-left corner that lets every click through, and shows
  who is speaking, "Muted", and a pill for two seconds when someone comes
  or goes. The game must run in borderless windowed mode; nothing is
  injected into it.
- The latency lab (spec section 11), out of a call: the loopback device
  test (the earphone held against the microphone; the median of nine
  clicks is input plus output latency, kept for the estimates), and the
  10-second voice gate calibration, controller in hand, which proposes a
  threshold 4 dB above the noise's loudest moments. In a call, each person
  shows an estimated mouth-to-ear latency, always labelled as an estimate
  (`src-tauri/src/latency.rs` says what it adds up and assumes), and a
  "Statistics" section lists the round trip, the device latency and each
  person's playback counters. None of it leaves the device.

The interface is in English and French; it follows the system's language
and can be switched in the header.

## How it is put together

- **Nothing in the interface sits on the audio path.** The audio callbacks
  belong to `drywire-core` and never allocate (ADR 0017). A session thread
  (`session.rs`) owns the call and the sound card; every 20 ms it copies
  the call's counters into a view, and the interface polls that copy a few
  times a second. No callback ever reaches JavaScript.
- **The commands are thin.** Each Tauri command (`commands.rs`) calls one
  method of `Desktop` (`app.rs`), so `tests/app.rs` drives the whole app
  against the real Go server, with in-memory sound cards.
- **Secrets stay sealed.** On Windows the identity and the space keys are
  encrypted with DPAPI for the current user. On macOS and Linux they are
  plain files readable by their owner only: development only (ADR 0031).
  The app keeps them in `%APPDATA%\com.lignenative.drywire` on Windows;
  `DRYWIRE_DESKTOP_HOME` moves them, to run two copies side by side.
- **The binary is `drywire-desktop`**, never `drywire`, which is the
  command-line client (ADR 0029). The installer and the window say
  "Drywire".

## Running it

You need what the core needs (Rust through `rustup`, CMake, a C compiler),
plus [Bun](https://bun.sh). Node.js is not used. On Linux, Tauri also needs
WebKitGTK and a few libraries:

```sh
sudo apt-get install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev
```

Then, from the repository root:

```sh
make dev       # in one terminal: a local server on 127.0.0.1:10050
make desktop   # in another: Vite on 127.0.0.1:10051, then the window
```

The Vite port is fixed and Vite refuses to start rather than move;
`DEV_PORT=10059 make desktop` moves both Vite and the window to another one.
To join the local server, create a space and an invitation with the
command-line client (see [`server/README.md`](../server/README.md)) and
paste the link in the app.

From `desktop/`:

```sh
bun install              # the interface's dependencies, from bun.lock
bun run dev              # Vite alone, for the interface (no Rust side)
bun run typecheck        # TypeScript
bun test                 # the interface's tests
bun run tauri build      # the app and its installer (NSIS on Windows)
```

`make test` runs the typecheck, the tests and the license check of every
JavaScript package (`scripts/js-licenses.ts`), then `cargo fmt`, `clippy`,
`test` and `deny` in `src-tauri/`.

## Testing on Windows

CI builds the installer on Windows and runs the Rust tests there, DPAPI
included; the installer is the `drywire-desktop-windows` artifact of each
run. What only a person on a Windows PC can check:

- [ ] **Install and start.** The unsigned installer runs (SmartScreen:
      "More info", "Run anyway"); the Start menu says Drywire.
- [ ] **DPAPI.** `%APPDATA%\com.lignenative.drywire\identity.json` says
      `"protection": "dpapi"` and its data is not the key the app shows.
      Copied into another Windows account's folder, the app refuses it
      with a clear message.
- [ ] **A real call.** Paste an invitation link, join a room on the
      default sound card, hear a teammate and be heard; leave, and the
      teammate sees you go at once.
- [ ] **Devices.** "Audio devices" shows WASAPI and small buffers; a
      Bluetooth headset is flagged.
- [ ] **Widget.** The meter moves with your voice and turns green above the
      gate's mark; muting stops your teammate hearing you within a word;
      a teammate at 200 % is clearly louder, at 0 % silent.
- [ ] **Global shortcut in a game.** With a game in the foreground (and in
      a game that runs as administrator: then Drywire must too), the mute
      shortcut mutes and unmutes; the overlay says "Muted".
- [ ] **Overlay over a borderless game.** The pill shows in the top-left
      corner over the game, the game keeps every click and the focus, and
      the frame rate does not change. In exclusive fullscreen the overlay
      cannot show: expected.
- [ ] **Tray.** Closing the window keeps the call; the tray icon brings it
      back; "Quit" leaves the room and the teammate sees it at once.
- [ ] **Presence sounds** are quiet, distinct, and never delay or duck a
      teammate's voice; at "Off" there are none.
- [ ] **Latency lab.** The device test hears its clicks with a wired
      earphone held to the microphone (expect tens of milliseconds) and
      shows far more with a Bluetooth headset. The calibration's proposed
      threshold keeps controller clicks out and lets normal speech in.
- [ ] **Real latency.** Measure a full mouth-to-ear path with the physical
      method of spec section 11 (two PCs side by side, a third recorder;
      the `drywire-bench measure` command it names is not written yet, so
      the gaps are read off the recording by hand), record it in
      `docs/measurements.md`, and compare it with the in-app estimate.
