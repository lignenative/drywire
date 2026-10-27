# 0004. Configure phase 1 rooms with a JSON file

Status: Accepted · 2026-10-27 · Superseded by ADR 0024

## Context

Phase 1 proves latency before the control plane exists, yet the relay must
already know which sessions belong to which room and check every `hello`
against a session secret. The server is standard library only.

## Decision

- Rooms come from a JSON file named by `DRYWIRE_ROOMS`: rooms, and for each
  member a session number and a 32-byte secret in hex. JSON because the
  standard library reads it; no YAML or TOML dependency.
- The relay verifies `hello` with these secrets from phase 1 on. Audio stays
  unencrypted only in the sense that the relay never decrypts it; the
  authenticated `hello` is relay-side and cheap, so it ships now instead of
  changing the protocol in phase 2.
- `server/rooms.dev.json` is committed for local development. Its secrets are
  public, so the file says `"dev_only": true` and the server refuses to serve
  it on anything but a loopback address.

## Consequences

- `make dev` works on a fresh clone with no setup, and nobody can expose the
  public secrets by mistake.
- A real test over the internet needs its own rooms file with fresh secrets
  (`openssl rand -hex 32` per member), kept out of the repository.
- In phase 2 the control plane replaces the file; the relay's internal model
  (rooms of sessions with secrets) does not change.
