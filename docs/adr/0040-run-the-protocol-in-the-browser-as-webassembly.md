# 0040. Run the protocol in the browser as WebAssembly, and Opus through WebCodecs for now

Status: Accepted · 2026-11-04

## Context

The spec plans the web client as "React + WASM core": the same Rust code
as every native client, in the browser. What the core holds, and what a
browser needs, part by part:

- **Protocol and encryption** (`drywire-proto`): packets, HKDF, AES-GCM,
  HMAC, Ed25519 verification, replay window. Pure Rust on RustCrypto and
  dalek. It builds for `wasm32-unknown-unknown` as it is, without
  `getrandom`: the workspace turns off `aes-gcm`'s default features, and
  the only randomness a client needs, its sender salt, can come from
  `crypto.getRandomValues`.
- **Opus**: `drywire-core` binds libopus through `opusic-sys` (ADR 0009),
  which compiles C with CMake. `wasm32-unknown-unknown` has no C library,
  and Apple's clang has no WebAssembly backend: building libopus for the
  web means Emscripten or the WASI SDK, a second toolchain.
- **Audio I/O, gate, jitter buffer, mixer** (`drywire-core`): the
  backends are the operating systems' APIs, and the crate as a whole
  does not build for the browser.
- **Identity**: an Ed25519 key the client never lets out (spec
  section 8).

What the browsers offer instead
([MDN compatibility data](https://github.com/mdn/browser-compat-data),
[Opus in WebCodecs](https://www.w3.org/TR/webcodecs-opus-codec-registration/)):

- **WebCodecs** `AudioEncoder` and `AudioDecoder` with Opus: Chrome and
  Edge 94 (Opus options from 110), Firefox 130 on the desktop (not on
  Android), Safari 26. Frames of 2.5, 5 and 10 ms are accepted by the
  three engines' code. But `application: "lowdelay"`, the CELT-only
  mode Drywire uses (spec section 7), is only honoured by Chrome;
  Firefox and Safari ignore it and encode in the default mode, with
  more lookahead. There is no way to ask the decoder to conceal a lost
  packet. And WebCodecs exists in windows and workers, not in an
  `AudioWorklet`, so every frame crosses a thread.
- **Pure-Rust Opus ports** appeared in 2026 (`opus-rs`, `opus-pure`,
  BSD-3), which would run inside the worklet with concealment. They are
  months old.
- **WebCrypto Ed25519**: Chrome 137, Firefox 129, Safari 17. A key made
  with `extractable: false` can be stored in IndexedDB and stays
  unreadable by script.
- **wasm-bindgen** generates JavaScript glue that leans on `TextDecoder`,
  which an `AudioWorklet` has historically lacked; and it is one more
  tool whose version must match the crate's.

## Decision

- **`drywire-wasm`**, a new crate in the core's workspace, compiles
  `drywire-proto` to WebAssembly for the web client. It holds a small
  client: it builds `hello`, checks `hello_ack`, builds the announcement
  from a signature, seals our audio, verifies others' announcements and
  opens their audio, with the replay window. Its only dependency is
  `drywire-proto`.
- **No wasm-bindgen.** The module exports plain C functions and one
  shared buffer in its memory; the page writes the input there, calls,
  and reads the output. It loads with `WebAssembly.instantiate` alone,
  in a page, a worker or a worklet, and needs no `unsafe`.
- **The identity key stays in WebCrypto**, non-extractable, in
  IndexedDB. The page signs the login and the announcement itself;
  WebAssembly only ever sees public keys. An epoch's announcement is
  signed once and sent again unchanged (Safari's Ed25519 signatures are
  randomised, which receivers do not mind).
- **Opus through WebCodecs, for now**, with `lowdelay` where the browser
  honours it. It is the shortest path to a working client. The next audio
  slice measures it against a pure-Rust Opus in the worklet, and the
  measurement decides (spec section 1: measured, never promised).
- `drywire-wasm` repeats, in a few lines, the receiving half of
  `drywire-core`'s `Inbound`. When `drywire-core`'s pure parts (gate,
  jitter buffer, inbound, sender) build without libopus and the audio
  backends, the web client uses them and that copy goes.

## Consequences

- The browser runs the exact protocol code the vectors check: the web
  client's tests read the same `core/drywire-proto/vectors/`.
- Building the web client needs the `wasm32-unknown-unknown` target
  (`rustup target add wasm32-unknown-unknown`), nothing else.
- Until the audio decision, a browser's audio has more delay than it
  needs: a thread hop each way, and in Firefox and Safari Opus's default
  mode. Firefox on Android has no WebCodecs audio and cannot be a client.
- `cargo deny` keeps to the native targets: `drywire-wasm` pulls in no
  crate the native build does not already check, while adding the
  WebAssembly target would bring in what other crates (the control
  plane's HTTPS client) would use on it, which no client builds.
