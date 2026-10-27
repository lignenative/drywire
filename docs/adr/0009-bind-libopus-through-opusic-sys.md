# 0009. Bind libopus through opusic-sys

Status: Accepted · 2026-10-27

## Context

The core encodes and decodes with libopus (BSD-3-Clause), in
`OPUS_APPLICATION_RESTRICTED_LOWDELAY` mode. We need a Rust binding that
builds libopus from source on every target (Windows first, then Apple,
Android and Linux), so that no developer or user installs it separately.

Candidates checked on 2026-10-27:

- `audiopus_sys` 0.2 (ISC): bundles libopus 1.3 from 2019, whose
  `CMakeLists.txt` asks for CMake 3.1. CMake 4 refuses it outright.
- `opus` 0.4 (MIT or Apache 2.0): a safe API on top of `audiopus_sys`, so
  the same problem, plus an API we would partly bypass for the controls and
  the concealment calls we need.
- `opusic-sys` 0.7 (BSD-3-Clause): raw bindings, bundles libopus 1.6.1,
  built with CMake through the `cmake` crate (MIT or Apache 2.0), with
  Android and MSVC handled. It patches the MSVC runtime selection and
  enables one optional module flag; nothing on the code path we use.
- A pure Rust Opus: none is mature enough to trust with every frame.

## Decision

- Depend on `opusic-sys` with its default `bundled` feature, and write our
  own small safe wrapper, `drywire-core/src/opus.rs`, exposing only what
  Drywire needs: encode, decode, concealment, lookahead, frame size of a
  packet. Every `unsafe` block carries a `SAFETY` comment (enforced by
  clippy's `undocumented_unsafe_blocks`).
- Building the core therefore needs CMake and a C compiler, in addition to
  `cargo`.

## Consequences

- One `cargo build` gives a statically linked libopus on every platform.
- The wrapper is about 200 lines anyone can read in one sitting, and its
  tests pin facts we rely on: every frame is CELT-only, and the encoder's
  lookahead is 120 samples (2.5 ms) at every frame size.
- Upgrading libopus means upgrading `opusic-sys`; its changes to the build
  must be read before each upgrade.
