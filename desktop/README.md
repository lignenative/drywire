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
- Joins a room on the default sound card, shows who is there and the ping
  to the relay, and leaves.

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
