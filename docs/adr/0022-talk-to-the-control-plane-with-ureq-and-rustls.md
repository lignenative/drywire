# 0022. Talk to the control plane with ureq and rustls

Status: Accepted · 2026-10-28

## Context

Every client talks to the control plane over HTTPS (protocol section 13):
`drywire-cli` now, the desktop app in phase 3, and the mobile apps through
the same Rust core. The server only accepts TLS 1.3. The client needs a
blocking HTTP client, TLS, a way to trust certificates, and a reader for
Server-Sent Events, all under the licenses of CLAUDE.md.

Options considered:

- **ureq with rustls**: small, blocking (no async runtime), MIT or
  Apache-2.0. rustls needs a crypto provider: `ring` (Apache-2.0 and ISC)
  or `aws-lc-rs` (which brings OpenSSL-licensed code).
- **ureq or reqwest with native-tls**: the operating system's TLS. macOS's
  SecureTransport and Windows 10's SChannel stop at TLS 1.2, so a client
  could not reach a TLS 1.3 server on those systems.
- **reqwest**: pulls in tokio and hyper, far more code than a client that
  makes a few requests a minute needs.
- **Writing HTTP/1.1 by hand over a TCP socket**: fine for plain HTTP, but
  TLS is not something to write by hand.

## Decision

- A new crate, `drywire-control`, is the control plane client: login,
  spaces, invitations, sessions, the event stream. It does no audio and
  `drywire-core` does not depend on it; an app holds both.
- It uses **ureq 3 with rustls on ring**, and **rustls-platform-verifier**
  to trust certificates: the operating system's store, so an operator's
  private CA or a corporate proxy works as it does in a browser, and no
  copy of the Mozilla list (`webpki-roots`, CDLA-licensed) is bundled.
- Plain `http://` is refused unless the instance is this machine.
- Server-Sent Events are parsed by hand (sixty lines): the format is
  simple, and no crate is needed for it.

## Consequences

- `ring` compiles C code, so checking `drywire-cli` for Windows from a Mac
  with `cargo clippy --target x86_64-pc-windows-msvc` no longer works
  without the Windows SDK headers. `make test` checks `drywire-core`'s
  library for Windows (the WASAPI backend, which is what that check is
  for), without its tests, which reach the server through
  `drywire-control`; the Windows CI job builds and tests everything
  natively.
- About 80 crates join the dependency tree of the CLI. `cargo deny` checks
  every one of them against the license list.
