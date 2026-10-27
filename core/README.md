# core

The Rust workspace shared by every Drywire client: the wire protocol, the
end-to-end encryption, Opus, the voice gate and the jitter buffer. Native
apps link it; nothing in it talks to the network or to a sound card yet.
Section 6 of [`docs/spec.md`](../docs/spec.md) says where it is going.

| Crate | What it does |
| --- | --- |
| [`drywire-proto`](drywire-proto/) | Packets, key derivation, AES-GCM, signed announcements, replay window. No I/O, no clock. |
| [`drywire-core`](drywire-core/) | Opus (libopus, CELT only), the voice gate, the jitter buffer, and the client pipeline built on them: config file, sender, receiver, mixer, relay link. No sound card, no socket. |
| [`drywire-bench`](drywire-bench/) | Replays simulated network traces against the jitter buffer (`make bench`); its regression test runs in `make test`. |

## Run the tests

From the repository root, `make test` runs `cargo fmt --check`, `cargo
clippy` and `cargo test` here after the Go suite. Inside `core/`:

```sh
cargo test
```

Building `drywire-core` compiles libopus from source, so it needs CMake and
a C compiler besides Rust (ADR 0009). Some integration tests start the real
Go relay (`testsupport/relay.rs` builds it with `go build`), so they need Go
too, or `DRYWIRE_SERVER_BIN` set to a built `drywire-server`. Rust comes from `rustup`; on a Mac with Homebrew's keg-only `rustup`, put
`/opt/homebrew/opt/rustup/bin` on your `PATH`.

## Where to start reading

[`docs/protocol.md`](../docs/protocol.md) first: `drywire-proto` follows it
section by section, and its comments cite the section numbers. Then
`drywire-proto/tests/vectors.rs`, which decodes the shared binary test
vectors in [`drywire-proto/vectors/`](drywire-proto/vectors/) exactly like the
Go relay's tests do. The two implementations share no code; agreeing on
those files is what proves they speak the same protocol.

## Dependencies

Declared once, in `Cargo.toml` under `[workspace.dependencies]`. Each one
is checked against the allowed licenses in [`CLAUDE.md`](../CLAUDE.md)
before it is added, with every crate it pulls in; ADR 0008 records the
current list.
