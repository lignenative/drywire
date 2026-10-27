# 0008. Use RustCrypto and dalek for the core's cryptography

Status: Accepted · 2026-10-27

## Context

`drywire-proto` needs HMAC-SHA256, HKDF-SHA256, AES-256-GCM and Ed25519
(protocol sections 6 and 8). The Rust options:

- **RustCrypto** (`aes-gcm`, `hkdf`, `hmac`, `sha2`) and **dalek**
  (`ed25519-dalek`): pure Rust, one small crate per primitive, MIT or
  Apache 2.0 (dalek: BSD-3-Clause). AES uses the CPU's AES instructions
  (AES-NI, ARMv8 crypto extensions), detected at run time.
- **ring**: fast and audited, but a large crate with C and assembly built by
  a build script, which complicates cross-compiling to every platform we
  target (Windows, Apple, Android, Linux, WebAssembly later).
- **aws-lc-rs**: needs CMake and a C toolchain; heavier still.

Speed is not the deciding factor: at 400 packets per second, encrypting one
80-byte frame takes well under a microsecond with hardware AES.

## Decision

- Use `aes-gcm`, `hkdf`, `hmac`, `sha2` and `ed25519-dalek`, declared once
  in `core/Cargo.toml` under `[workspace.dependencies]`.
- `aes-gcm` is built without default features: we need neither its random
  nonce generation (our nonce is the sequence number) nor `getrandom`.
- Every transitive crate was checked against the allowed licenses. All are
  MIT, Apache 2.0, BSD or ISC, with one addition: `unicode-ident` is
  `(MIT OR Apache-2.0) AND Unicode-3.0`. It is pulled in at build time by
  `proc-macro2`, which every Rust procedural macro uses (here through
  `curve25519-dalek-derive`), and its Unicode-3.0 part covers the Unicode
  character tables. Unicode-3.0 is a permissive, OSI-approved license, so we
  allow it, for that use, in `CLAUDE.md`.

## Consequences

- The core builds with nothing but `cargo` for the cryptography; no C
  toolchain is needed until Opus.
- When CI exists, `cargo-deny` enforces the same list automatically.
