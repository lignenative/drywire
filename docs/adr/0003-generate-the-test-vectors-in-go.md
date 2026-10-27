# 0003. Generate the shared test vectors in Go

Status: Accepted · 2026-10-27

## Context

The Go relay and the Rust core share no code; binary test vectors are their
contract. Someone has to produce those vectors, and hand-written binary files
are error-prone and hard to review.

## Decision

- A Go command, `server/cmd/drywire-vectors` (`make vectors`), writes the
  `.bin` files and `manifest.json` into `core/drywire-proto/vectors/` from
  fixed, recognisable inputs.
- The end-to-end crypto and the audio framing, which the relay never needs,
  get a small reference implementation in `server/internal/clientref`, used
  only by the generator and the tests.
- A Go test fails when the files on disk differ from the generator's output;
  another decodes every file and compares it with the manifest.
- The hand-written header example of `docs/protocol.md` is checked by a unit
  test, and the key derivations were checked once against an independent
  HKDF implementation, so the vectors do not merely agree with themselves.

## Consequences

- The Go standard library (HKDF, AES-GCM, Ed25519, HMAC) is the reference
  for every primitive.
- The Rust tests only read the vectors; they never write them. A mismatch
  between Go and Rust is a bug to understand, not a file to regenerate.
- Changing the wire format means changing the document, the generator, the
  vectors and both implementations in one commit.
