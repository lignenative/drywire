# Drywire

Drywire is an open source voice chat for teams who play together and want
the lowest possible delay between one player's mouth and another player's
ear. It sends untouched voice (no noise suppression, no automatic gain, no
echo cancellation), encrypted end to end, through a small relay server that
forwards packets without being able to read them.

It is voice only: no text chat, no video, no screen sharing.

> **Status: early development.** Phase 1 (the low-level prototype) is under
> way. There is no app to install yet. Every latency figure in the docs is a
> hypothesis until a measurement replaces it.

## How it works

```text
 client A ──UDP──▶ relay ──UDP──▶ client B
 (Rust core)       (Go)          (Rust core)
```

- **Clients** capture the microphone, encode it with Opus in its lowest-delay
  mode, encrypt it, and send small UDP packets every 2.5, 5 or 10 ms. Every
  client shares one Rust core; only the interface differs per platform.
- **The relay** (`server/`) receives each packet and copies it to the other
  people in the same room. It never decrypts or decodes audio.
- **The wire format** is written down in [`docs/protocol.md`](docs/protocol.md)
  and checked by binary test vectors that the Go and Rust tests both read.

The full design, its reasons and the roadmap are in
[`docs/spec.md`](docs/spec.md).

## Repository layout

| Path | What it holds |
| --- | --- |
| `server/` | The Go relay and its HTTP API ([`server/README.md`](server/README.md)) |
| `core/` | The Rust core shared by every client, and `drywire-cli`, the phase 1 client ([`core/README.md`](core/README.md)) |
| `docs/` | The specification, the protocol, and decision records (`docs/adr/`) |
| `scripts/` | The scripts behind every `make` command |
| `Makefile` | Entry point for every command; `make help` lists them |
| `.github/` | Continuous integration: Linux, macOS and Windows builds and tests, license checks |

More directories (`apps/`, `site/`, `deploy/`) appear as each phase starts.
The target layout is in section 12 of the specification.

## Getting started

You need:

- **macOS or Linux** with `make` and a POSIX shell (Windows: see
  [`core/drywire-cli/README.md`](core/drywire-cli/README.md)).
- **Go 1.27** or newer, for the relay.
- **Rust** through `rustup`, plus **CMake** and a C compiler, for the core
  (it builds libopus from source).

Then:

```sh
make help      # list the commands
make test      # run every test suite
make fmt       # format all code
make dev       # run the relay locally with the development rooms
make live      # talk to an echo bot through a local relay (headphones)
make vectors   # regenerate the protocol test vectors after a format change
make bench     # replay simulated network traces against the jitter buffer
```

`make dev` needs no setup: it serves `server/rooms.dev.json`, whose secrets
are public and which the server only accepts on a loopback address. Check it
with `curl http://127.0.0.1:10050/v1/status`.

Local ports follow a fixed block, bound to `127.0.0.1` only:

| Port | Service | Override |
| --- | --- | --- |
| 10050/tcp | Server HTTP API | `DRYWIRE_HTTP_ADDR` |
| 10050/udp | Relay | `DRYWIRE_UDP_ADDR` |

A port that is already taken stops the server with an error; it never moves
to another port on its own. To override a value, copy `.env.example` to
`.env` and edit it.

## Contributing

Read [`CLAUDE.md`](CLAUDE.md) first: it lists the principles, the allowed
dependency licenses and the conventions that every change follows, for people
and for coding agents alike.

## License

The code is released under the [MIT license](LICENSE). The Drywire name and
logo belong to Ligne Native and are not covered by the license.
