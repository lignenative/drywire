# Contributing to Drywire

Thank you for wanting to help. Drywire is a voice chat built for the lowest
possible latency, and it is meant to stay small and easy to read. This page
says how to get the code running and what a change needs before it is
merged.

## Before you start

- Read [`docs/spec.md`](docs/spec.md) for what we build and why, and
  [`CLAUDE.md`](CLAUDE.md) for how: the working rules there apply to every
  contributor, human or not.
- For anything larger than a fix, open an issue first and describe what
  you want to change. Some ideas are out of scope on purpose (text chat,
  video, noise suppression: spec section 5), and it is better to know
  before writing code.
- Security problems are not reported in issues: see
  [`SECURITY.md`](SECURITY.md).

## Set up

You need Go, Rust (through `rustup`), CMake and a C compiler; the root
[`README.md`](README.md) has the details. Then:

```sh
make test   # everything that CI runs, in a minute or two
make live   # a live call with an echo bot on your Mac (headphones)
```

Each top-level directory has a `README.md` that says what it holds and how
to run it.

## What a change needs

- **Tests.** `make test` passes. A bug fix comes with the test that would
  have caught it; new behaviour comes with tests of its own.
- **Latency first.** A change on the audio path says what it does to
  mouth-to-ear latency. A measured regression is a bug. Latency figures in
  docs, commits or the app come from a recorded measurement
  ([`docs/measurements.md`](docs/measurements.md)), never from a guess.
- **Untouched voice.** No noise suppression, gain control or echo
  cancellation, ever. The voice gate decides when to send; it never
  changes samples.
- **The wire format has one source of truth**,
  [`docs/protocol.md`](docs/protocol.md). Changing it means changing the
  document, the binary vectors (`make vectors`) and both implementations,
  Go and Rust, in the same change.
- **Decisions are recorded.** A significant choice (a dependency, a
  design trade-off, a change of plan) gets a short ADR in
  [`docs/adr/`](docs/adr/), numbered in order.
- **Few dependencies, permissive licenses.** Allowed: MIT, BSD, Apache 2.0,
  ISC, Zlib, MPL 2.0 (and Unicode-3.0 for `unicode-ident`). Never GPL,
  LGPL or AGPL. The Go server uses the standard library only, apart from
  the dependencies the spec accepts; anything else needs an ADR first.
  CI checks Rust licenses with `cargo deny`.
- **English** in code, comments, commits and documentation. Comments
  explain why, not what.
- **No secret and no private detail** in the repository: no real server
  name, address, or person. Examples use generic people and hosts.

## Commits and pull requests

- One logical change per commit; the build and the tests pass at every
  commit.
- The subject line is short and imperative ("Add the voice gate"), the body
  explains why.
- A pull request says what it changes, why, and how it was tested, with
  any measurement it relies on.

By contributing, you agree that your contribution is licensed under the
project's [MIT license](LICENSE).
