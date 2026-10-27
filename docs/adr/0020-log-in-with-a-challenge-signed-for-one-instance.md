# 0020. Log in with a challenge signed for one instance

Status: Accepted · 2026-10-27

## Context

Drywire has no accounts and no passwords: each device holds an Ed25519 key
pair (spec section 8). The control plane must know which identity sends a
request, and a client can talk to several instances, some of which may be
run by people it should not trust with its identity.

## Decision

- A client asks for a random 32-byte challenge, signs
  `"drywire/login/v1" ‖ challenge ‖ audience` with its identity key, and
  gets an opaque bearer token valid for one hour (protocol section 13.2).
- The **audience** is the instance's canonical origin as the client reached
  it, and the server checks it against its own (`DRYWIRE_PUBLIC_URL`).
  Without it, a malicious instance could forward another instance's
  challenge to a visiting client and log in there as that client.
- The prefix keeps login signatures apart from announcement signatures,
  which use the same key (protocol section 8.3).
- Challenges are single-use, live one minute, and at most 10,000 wait at a
  time. Every failed login gets the same answer.
- Challenges and tokens live **in memory only**, indexed by the token's
  SHA-256. A restart logs everyone out, which costs a client one round
  trip, and no token is ever written to disk or to a backup.
- An unknown key may log in, but the server stores nothing about it until
  it creates a space or redeems an invitation, with the display name it
  gives then. Logging in updates `last_seen_at`, which drives the
  inactivity purge.
- The login message and the invitation link have vectors in the shared
  manifest (ADR 0003), so the Go server and the Rust clients are checked
  against the same bytes.

## Consequences

- A stolen token is worth an hour at most, and the private key never
  leaves the device.
- Several servers behind one public URL would need a shared token store;
  one server per instance is the model until then.
- A client must canonicalise the instance address exactly as the server
  does; both implementations are tested on the same cases.
