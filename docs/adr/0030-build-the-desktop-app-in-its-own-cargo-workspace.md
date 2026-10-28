# 0030. Build the desktop app in its own Cargo workspace

Status: Accepted · 2026-10-28

## Context

Phase 3 is the Windows app, in Tauri v2 and React (spec section 10). Its
Rust side links `drywire-core` and `drywire-control`. The question is
where its crate lives:

- **In the `core/` workspace.** One `Cargo.lock` and one target directory
  for every Rust crate. But Tauri brings about 470 crates, and on Linux
  the WebKitGTK headers: every `cargo clippy` and `cargo test` of the
  core, on every contributor's machine and CI runner, would build them,
  and a Linux machine without WebKitGTK could no longer test the core at
  all. `core/deny.toml` would also have to accept what Tauri needs.
- **A workspace of its own, in `desktop/src-tauri/`.** The core stays a
  small set of crates with few dependencies; the app pays for Tauri
  alone. Two lock files can resolve shared crates (`serde`, `rustls`) to
  different versions, which only costs build time.

The spec's layout put the app in `apps/desktop/`. With one app in the
repository so far, a top-level `desktop/` next to `server/` and `core/`
is easier to find, and each top-level directory has its README.

ADR 0029 also requires that no other binary be called `drywire`: on the
case-insensitive file systems of Windows and macOS, a Tauri binary named
`Drywire` would overwrite the command-line client's in a shared target
directory.

## Decision

- The app lives in `desktop/`: the React interface at its root, built by
  Vite and run by Bun, and the Tauri crate in `desktop/src-tauri/`, a
  Cargo workspace of its own with its own `Cargo.lock` and `deny.toml`.
  It depends on the core's crates by path.
- The crate and its binary are named `drywire-desktop`, and Tauri's
  `mainBinaryName` keeps that name in the installer. Only the product
  name, the window and the installer say "Drywire".
- `make test` and CI check it as a separate component: typecheck, tests
  and license check of the interface with Bun, then `cargo fmt`, `clippy`,
  `test` and `deny` in `desktop/src-tauri/`.
- `deny.toml` accepts exactly what `core/deny.toml` accepts, with one
  exception: `target-lexicon`, a build-time dependency of the Linux GTK
  bindings, is "Apache-2.0 WITH LLVM-exception". The exception only
  removes Apache 2.0's attribution requirement for compiled code; it adds
  no obligation, so the crate is allowed by name, and nothing else under
  that license is.

## Consequences

- A change to the core's public API can break the desktop app without
  breaking `cargo test` in `core/`; `make test` runs both, so it shows.
- The two target directories never mix, so even a `Drywire` binary could
  not overwrite `drywire.exe`; the name rule holds anyway.
- On Linux, `make test` skips the app with a note when WebKitGTK is not
  installed, and checks the rest.
- The spec's layout is updated: `desktop/` instead of `apps/desktop/`.
  The Apple, Android and web apps choose their place when they start.
