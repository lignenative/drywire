# 0006. Encrypt audio from phase 1

Status: Accepted · 2026-10-27 · Its configuration file is superseded by ADR 0024

## Context

The specification's first draft said phase 1 would carry "no encryption
yet", and that end-to-end encryption would arrive with the control plane in
phase 2. Meanwhile the protocol (`docs/protocol.md`) was written with
encryption built in: every audio packet is AES-256-GCM, and the format has no
plaintext mode.

Keeping a plaintext mode for one phase would mean a second payload layout, a
flag or a version to tell them apart, and code paths in the relay, the
clients and the vectors that exist only to be deleted later. It would also
make phase 1's latency measurements wrong: phase 2's exit criterion is
"latency within 1 ms of phase 1", which only means something if phase 1
already pays the cost of encryption.

What phase 1 really lacks is not encryption but a way to distribute keys:
there is no control plane and no invitation link yet.

## Decision

- Audio is encrypted exactly as the protocol describes, from the first
  commit of the Rust core. There is no plaintext mode, in phase 1 or later.
- In phase 1, what the control plane and the invitation link will provide
  comes from a client configuration file instead: the `space_key`, the
  `room_id`, the client's own Ed25519 seed, and the session number and public
  key of every member of the room.
- The development configuration committed to the repository uses public
  keys and is marked `"dev_only": true`, like `server/rooms.dev.json`
  (ADR 0004). A real test over the internet uses a configuration with fresh
  random keys, kept out of the repository.
- This supersedes the sentence of ADR 0004 that called phase 1 audio
  "unencrypted only in the sense that the relay never decrypts it": audio is
  encrypted end to end in phase 1, with keys from a file.

## Consequences

- Phase 1 measures the real data path, encryption included, so phase 2's
  latency criterion compares like with like.
- Phase 2 replaces the configuration file with the control plane and the
  invitation link. The packet format, the key derivation and the code that
  encrypts and decrypts do not change.
- A phase 1 configuration file holds secrets. It must be treated like a
  password: never committed unless it is a public development file.
