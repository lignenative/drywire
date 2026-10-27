# server

`drywire-server`, the Go side of Drywire. Standard library only.

| Package | What it does |
| --- | --- |
| `internal/wire` | Encodes and decodes packets as the relay sees them: header, control packets, replay window |
| `internal/clientref` | Reference of what clients do above the relay (key derivation, audio encryption, payload framing); used only to build and check test vectors |
| `internal/vectors` | Builds, writes and reads the shared test vectors |
| `cmd/drywire-vectors` | Writes the vectors to `core/drywire-proto/vectors/` (`make vectors`) |

Everything follows [`docs/protocol.md`](../docs/protocol.md); comments refer
to its section numbers.

```sh
go test ./...   # or `make test` from the repository root
```
