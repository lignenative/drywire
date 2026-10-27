# Protocol test vectors

Binary packets that every implementation of
[`docs/protocol.md`](../../../docs/protocol.md) must decode exactly as
`manifest.json` describes. They are the contract between the Go relay and the
Rust core, which share no code.

- One `.bin` file per packet, valid or deliberately broken.
- `manifest.json` lists the keys and secrets used (`inputs`, `epochs`), and
  for each file either the decoded fields or the error code a decoder must
  report. Section 12 of the protocol describes its format.
- Every key is a run of consecutive bytes (`000102…`), so the vectors are
  reproducible and easy to recognise in a hex dump. Never use them as real
  keys.

Do not edit these files by hand. They are written by
`server/cmd/drywire-vectors`:

```sh
make vectors
```

A Go test (`server/internal/vectors`) fails whenever the files differ from
what the generator writes, and another decodes each file and compares it with
the manifest. `core/drywire-proto/tests/vectors.rs` does the same from the
Rust side, and also re-encodes every valid packet.
