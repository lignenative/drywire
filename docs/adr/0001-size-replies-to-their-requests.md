# 0001. Size every reply to its request

Status: Accepted · 2026-10-27

## Context

A UDP server that answers a small packet with a larger one can be abused to
flood a third party: the attacker forges the victim's address as the source,
and the server amplifies the traffic. The relay must answer `hello` and
`ping`, so it needs a rule that makes this impossible by construction.

## Decision

- Every control packet has one exact size. A packet of any other size is
  invalid.
- `hello` and `hello_ack` share one layout (56 bytes). `ping` carries eight
  zero bytes so that it is exactly as large as `pong` (32 bytes).
- The relay sends nothing to an address before a valid `hello` from it, and
  never answers a packet it cannot verify, including one with an unknown
  protocol version.

## Consequences

- Reflection through the relay gains an attacker nothing, and an unknown
  address gets no answer at all.
- Extending a control packet means a new type or a new protocol version, not
  extra trailing bytes. That is a small price for a format that is easy to
  check.
- A client cannot learn over UDP that its version is unsupported. It learns it
  from the HTTP API instead (`GET /v1/version`).
