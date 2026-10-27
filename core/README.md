# core

The Rust workspace shared by every Drywire client: audio input and output,
Opus, the voice gate, the jitter buffer, the wire protocol and the end-to-end
encryption. It is not written yet; see section 6 of
[`docs/spec.md`](../docs/spec.md) for what it will hold.

What already lives here:

- [`drywire-proto/vectors/`](drywire-proto/vectors/): the binary test vectors
  of the wire protocol, which the Go relay's tests read today and the Rust
  `drywire-proto` crate will read too.
