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

- The window follows the design canvas's final page (CLAUDE.md, "Design
  first"). Wide, a permanent menu on the left lists the spaces this device
  knows: a space with one room is a single row named after the space, a
  space with several lists its rooms under its name, and each room shows
  who sits in it. Under the spaces, the call this device is in (one click
  back to it, one to leave), then its own bar: name, mute, settings.
- Beside the menu, the room picked in it. Out of a call: who is there and
  a button to join (joining another room leaves the current one). In a
  call: everyone's name in large, with bars that move while they speak,
  and three controls, the voice gate, mute and leave. A click on a name
  opens that person: the estimated mouth-to-ear latency (always labelled
  as an estimate), their volume from 0 to 200 % and their playback
  counters. A click on your own name or on the gate's button opens the
  level meter with the gate's threshold on it, as a white mark to drag. A
  click on the signal chip opens the connection: the round trip to the
  relay, this device's latency, what is lost on the way, and the audio
  devices. Connecting, reconnecting and a Bluetooth device each say so in
  a line above the names; a call the server ended offers to join again.
- A lock next to the space's name says whether the call is end-to-end
  verified: closed when everyone heard is verified, open while someone is
  being checked, open and amber when a voice is rejected (its audio is
  dropped, and its name dimmed). A click lists who is verified. The Rust
  side on `main` does not report it yet, so the lock is shown only in the
  simulated call until the branch that adds it is merged.
- Narrow (under 640 px), the spaces are the first screen and a bar at the
  bottom keeps the call one click away; a click on a room joins it.
- Accepts an invitation link and the name to show in that space. With no
  space yet, the window asks for one.
- The settings have their own menu: Voice (the gate's threshold and its
  10-second calibration, the mute shortcut), Sounds, Display (the overlay
  and its corner, the language, starting with the session), and Advanced
  (the loopback device test, and the device key an operator lists to let
  the device create spaces). Every setting applies at once. The gate and
  the volumes are remembered (`settings.json`, by public key for
  volumes); mute is not.
- Out of the window, for a player in a match: a tray icon (show, mute,
  quit; closing the window only hides it), a global mute shortcut
  (`Ctrl+Shift+M` by default, changed in the settings), three short
  presence sounds when someone joins, leaves or loses their connection
  (synthesised by the interface, played through the web view's own output,
  never through the call), and the overlay: a transparent, always-on-top
  window in a corner of the main screen (top left unless the settings
  say another) that lets every click through, and shows
  who is speaking, "Muted", and a pill for two seconds when someone comes
  or goes. The game must run in borderless windowed mode; nothing is
  injected into it.
- When the network drops long enough for the server to forget the
  session, or the server restarts, the call shows "reconnecting" and
  joins the room again on its own, keeping mute, the gate and the volumes
  (ADR 0035).
- Starting with the session, hidden in the tray, if the person ticks it
  in the settings; off by default. On Windows this is a value under the
  current user's `Run` registry key, removed when unticked (untick it
  before uninstalling); on macOS, a launch agent.
- The latency lab (spec section 11), out of a call, in the settings: the
  loopback device test (the earphone held against the microphone; the
  median of nine clicks is input plus output latency, kept for the
  estimates), and the 10-second voice gate calibration, controller in
  hand, which proposes a threshold 4 dB above the noise's loudest moments.
  The estimates (`src-tauri/src/latency.rs` says what they add up and
  assume), the round trip and the playback counters stay on the device.

The interface is in English and French; it follows the system's language
and can be switched in the settings (Display).

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
- **The interface is built from shadcn components on Base UI**
  (`src/components/ui/`), themed in `src/index.css` from the design
  canvas (ADR 0037). Drywire's screens (`src/components/`) are made of
  them; `src/state.ts` holds the window's state and `src/model.ts` what
  the screens derive from the Rust side's views, tested without a
  browser. Where the Rust side does not report something the canvas shows
  yet (who sits in a room you are not in, the reconnection countdown, the
  microphone's level out of a call), `src/api.ts` marks the field
  optional with a TODO and the interface leaves it out.
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

### The interface alone, in a browser

```sh
make ui        # Vite on 127.0.0.1:10051; open it in any browser
```

Outside the Tauri window, `src/api.ts` answers every command from
`src/simulated.ts`: the canvas's spaces, people who talk, a voice gate
that opens, presence sounds. Nothing there is measured; it exists to check
the interface against the design canvas (CLAUDE.md, "Design first")
without building the Rust side or starting a server. `?scene=` in the
address opens one of the canvas's states: `call`, `muted`, `alone`,
`connecting`, `reconnecting`, `ended`, `poor`, `e2eePending`,
`e2eeFailed`, `firstRun`, `empty` or `unreachable` (the default is out of a call). The overlay is at
`/overlay.html?scene=call`. Compare the wide window at 1120 × 700 and
the narrow one at 380 × 640.

From `desktop/`:

```sh
bun install              # the interface's dependencies, from bun.lock
bun run dev              # Vite alone: the interface on a simulated call
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
- [ ] **Devices.** The connection (the signal chip) shows WASAPI and small buffers; a
      Bluetooth headset is flagged.
- [ ] **Call.** The voice gate's meter moves with your voice and turns green
      above the white mark; muting stops your teammate hearing you within a word;
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
