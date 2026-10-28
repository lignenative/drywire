# 0029. Name the command-line client `drywire`

Status: Accepted · 2026-10-28

## Context

The command-line client was built as `drywire-cli`, the name of its crate.
It has become a tool people type every day, not only a test harness: an
operator prints a device key with it, creates the first space and its
invitations; a player joins a room with it until the apps exist. A
project's main command usually carries the project's name alone (`git`,
`docker`, `tailscale`), and the `-cli` suffix says something the user
already knows: they are typing it in a terminal.

## Decision

The command is **`drywire`**. The crate and its directory keep the name
`drywire-cli` (`core/drywire-cli/`, `cargo build -p drywire-cli`), which
says what the crate is, next to `drywire-core` and `drywire-bench`;
`[[bin]] name = "drywire"` in its `Cargo.toml` names the binary. The
server keeps `drywire-server`.

Earlier ADRs keep the old name: they record what was decided then.

## Consequences

- The binary is `target/release/drywire` (`drywire.exe` on Windows, in
  the CI's `drywire-windows` artifact).
- The desktop apps must never build a binary named `drywire` in the same
  Cargo target directory: on Windows and macOS the file system ignores
  case, so a Tauri binary named `Drywire` would overwrite this one. The
  app's crate binary takes another name (such as `drywire-desktop`);
  only its installer and window show "Drywire".
- On Linux, where a packaged desktop app could also want `drywire` on the
  PATH, the choice comes back with the Linux phase.
