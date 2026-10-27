# 0024. Replace the phase 1 files with the control plane

Status: Accepted · 2026-10-28 · Supersedes ADR 0004, and the configuration
file of ADR 0006

## Context

Phase 1 took the relay's rooms from a JSON file (ADR 0004), and each
client's keys, session and fellow members from another (ADR 0006), with
public development copies committed and marked `dev_only`. Phase 2 brings
the control plane (protocol section 13) and invitation links (section 14),
which hand out the same things: sessions and secrets from the server, the
space key from the link, public keys from the join answer and the event
stream.

## Decision

- The relay's rooms file, the client configuration files, the committed
  development copies and `drywire-cli new-room` are removed. There is one
  way into a room: the control plane.
- `make dev` starts a server with a database in `.dev/` that lets anyone
  create a space; `make live` makes a space and an invitation link on a
  fresh server, and its echo bot joins with the link.
- The Rust integration tests get their sessions the way real clients do:
  `testsupport/relay.rs` creates a space, invites, redeems and joins
  through `drywire-control` against the real server.
- A client's identity and space keys live on the device (`drywire-cli`:
  `identity.json` and `spaces.json`, mode 0600). Nothing secret is
  committed any more.
- `drywire-core`'s `ClientConfig` stays, without its file format, as the
  parameters a call starts from.

## Consequences

- Encryption, packets and the relay's model (rooms of sessions with
  secrets) are unchanged, as ADR 0006 intended: phase 2's latency compares
  like with like.
- Every test that crosses the relay also crosses the control plane, so it
  needs Go and builds the server, as before.
- A member who joins mid-call can have their very first announcement
  arrive before the event that names them; it is dropped as unverifiable
  and repeated before their first talk spurt (protocol section 8.3).
