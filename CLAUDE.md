# Working on Drywire

This file is for anyone changing the code, human or coding agent. Read
[`docs/spec.md`](docs/spec.md) for what we build and why; this file says how.
When the code and a document disagree, the code wins and the document gets
fixed in the same change.

## Principles

- **Latency first.** Every change on the audio path is judged by its effect on
  mouth-to-ear latency. A measured regression is a bug.
- **Measured, never promised.** No latency figure in the docs, the app or a
  commit message unless a recorded measurement backs it.
- **Untouched voice.** No noise suppression, gain control or echo
  cancellation. The voice gate decides when to send, never alters samples.
- **The relay is blind.** It forwards encrypted packets; it never decrypts,
  decodes or stores audio.
- **Small and durable.** Standard library first. Few dependencies.
- **Easy to read.** A new developer should understand any package within a
  day. Prefer plain, explicit code over clever abstractions.

## Dependencies and licenses

- Allowed licenses: MIT, BSD, Apache 2.0, ISC, Zlib, MPL 2.0, and Unicode-3.0
  for the Unicode tables that `unicode-ident` brings to every proc macro
  (ADR 0008).
- Forbidden: GPL, LGPL, AGPL, and anything without a clear license.
- Go server: standard library only, except `modernc.org/sqlite` (control
  plane) and `quic-go` (web client), both accepted in the spec. Any other
  dependency needs an ADR first.
- Rust core: every new crate is checked against the list above; `cargo deny`
  (`core/deny.toml`) enforces it in CI, and in `make test` when installed.
- JavaScript: Bun is the package manager, script runner and test runner.
  Node.js is not used.

## Conventions

- **English everywhere** in the repository: code, comments, commit messages,
  documentation. The apps' interface is in English and French.
- **Written for newcomers.** Each top-level directory has a `README.md` that
  says what it does and how to run it. Comments explain *why*, not *what*.
- **Decisions are recorded.** Every significant choice gets a short ADR in
  [`docs/adr/`](docs/adr/), numbered in order.
- **The wire format has one source of truth**:
  [`docs/protocol.md`](docs/protocol.md). The binary vectors in
  `core/drywire-proto/vectors/` are read by both the Go and the Rust tests.
  Changing the format means changing the document, the vectors and both
  implementations together.
- **Semantic versioning.** The protocol version byte changes on every
  breaking change to the wire format.
- **Version skew is expected.** The server accepts the current and the
  previous protocol version. A client facing an unsupported version says an
  update is needed instead of failing silently.
- **No secret in the repository.** Configuration comes from environment
  variables and untracked files (`.env`). Development secrets that are
  committed live in files marked `"dev_only": true`, which the server refuses
  to serve on anything but a loopback address.
- **No private detail in the repository.** Examples use generic people,
  devices and hosts. Never commit a real server name, address or player.
- **Logs stay minimal.** Application logs never contain audio, tokens, secrets
  or IP addresses.

## Local ports

Development ports are fixed, bound to `127.0.0.1` only, and never fall back
silently to another port. Each one can be moved with an environment variable.

| Port | Service | Variable |
| --- | --- | --- |
| 10050/tcp | Server HTTP API | `DRYWIRE_HTTP_ADDR` |
| 10050/udp | Relay | `DRYWIRE_UDP_ADDR` |
| 10051 | Vite dev server, desktop UI (later) | `DEV_PORT` |
| 10052 | Project site preview (later) | `SITE_PORT` |
| 10057 | Vite dev server, web client (later) | `WEB_PORT` |

## Commands and tests

The Makefile is thin: each target calls a script in `scripts/`.

```sh
make help     # list the commands
make test     # gofmt, go vet, go test -race; cargo fmt, clippy, test; Windows clippy; cargo deny
make fmt      # gofmt and cargo fmt
make dev      # run the relay on 127.0.0.1:10050
make live     # live call on this Mac: you, an echo bot, a local relay
make vectors  # regenerate core/drywire-proto/vectors/ after a wire format change
make bench    # replay simulated network traces against the jitter buffer
```

`make test` must pass before every commit.

## Working rules

- **Code, test, iterate.** The spec is the plan; work in small vertical slices
  that each end with passing tests, rather than writing separate plan
  documents.
- **Atomic commits.** One logical change per commit, and the build and tests
  pass at every commit. The subject line is imperative and short; the body
  explains why.
- **Keep the git identity** configured in the repository; do not change the
  author email.
- **Ask before anything outward-facing**: creating the GitHub repository,
  pushing, publishing, or running any command on a remote server.
- **Leave a trail.** At the end of a working session, the repository (commits,
  docs, ADRs) must be enough for the next person to carry on.
