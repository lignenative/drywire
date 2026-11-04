# 0048. Share the desktop interface with the web client

Status: Accepted · 2026-11-04

## Context

The web client (`apps/web/`) proved the path from a browser to the relay
and back (ADR 0038 to 0040) with a developer's page. It now needs the
real interface. The desktop app already has it: React components drawn
from the design canvas (`desktop/src/`, ADR 0037), which talk to the
rest of the app through one typed object, `Api` (`desktop/src/api.ts`),
with two implementations so far: the Rust side in the Tauri window, and
a simulated call in a plain browser (`make ui`).

Copying the components into `apps/web/` would make two interfaces that
drift apart, and every change to the canvas would have to be built
twice. Moving them into a shared package would move every file under
`desktop/src/` while other work edits them, and add a workspace layer
that Bun, Vite, TypeScript and `make test` would all have to learn.

## Decision

- The web client imports the desktop interface where it is. Its Vite
  configuration and its `tsconfig.json` point `@/…` at `desktop/src/`,
  as the desktop app's own do, and resolve every package both use from
  `apps/web/node_modules` (Vite's `dedupe`, a TypeScript `paths`
  fallback), so the page holds one React and one Base UI. The web client
  declares the same packages at the same versions as the desktop
  interface.
- `Api` gets a third implementation, for the browser
  (`apps/web/src/api.ts`), built on the control plane over `fetch`, the
  relay over WebTransport, the protocol in WebAssembly, Opus in WebCodecs
  and two AudioWorklets. `desktop/src/api.ts` turns its `api` constant
  into a binding the web client replaces before the first render
  (`provideApi`), and says where the interface runs (`platform`: `app`
  or `web`).
- What a browser cannot do is left out by the shared components when
  `platform` is `web`: the mute shortcut (it would not work while a game
  has the focus), the overlay and its corner, launching at login, and the
  loopback test, replaced by a line on what the browser costs. The
  screens only the web client has (an unsupported browser, the
  microphone's permission) are its own components, in `apps/web/src/`,
  built from the desktop interface's parts. All of them are drawn on the
  canvas's V3 page first, in its "Navigateur · client web" row.
- The audio follows the native core's rules without its code, which does
  not build for the browser yet: the voice gate and its calibration
  (`gate.rs`), the jitter buffer's principles (`jitter.rs`, on decoded
  samples, since WebCodecs decodes on the main thread) and the mixer's
  soft clip, each tested against the core's own cases.

## Consequences

- A change to the interface lands once and shows in both clients; `make
  test` typechecks and builds the web client against the desktop files,
  so a change that breaks the browser fails there.
- Edits to `desktop/src/` for the web stay small: a flag read in a few
  places. A screen that only a browser has never goes into
  `desktop/src/`.
- The two `package.json` files must list the shared packages at the same
  versions; a mismatch would bundle what the web's lock file says, not
  what the desktop was tested with.
- When `drywire-core`'s pure parts build for WebAssembly, the gate, the
  jitter buffer and the mixer move into `drywire-wasm` and the
  TypeScript copies go away (ADR 0040 already plans it for the receiving
  half).
