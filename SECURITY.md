# Security policy

Drywire encrypts audio end to end, and its relay is designed never to hold
a key that decrypts it ([`docs/protocol.md`](docs/protocol.md), section 8).
We take any weakness in that design or its code seriously.

## Reporting a vulnerability

Please do not open a public issue. Report it privately through GitHub's
private vulnerability reporting: on the repository page, **Security**, then
**Report a vulnerability**. Include what you found, how to reproduce it,
and what an attacker could do with it.

We answer within a week, keep you informed while we fix it, and credit you
in the release notes unless you prefer otherwise. Please give us a
reasonable time to release a fix before you publish anything.

## Scope

In scope, among others:

- the relay (`server/`): amplification, address spoofing, binding a
  session without its secret, resource exhaustion, anything that lets it
  read audio;
- the protocol and the core's cryptography (`core/drywire-proto/`): key
  derivation, nonce reuse, replay, forged announcements;
- the clients: anything that leaks keys, audio or personal data.

Not vulnerabilities:

- A development server (`make dev`) lets anyone create a space and
  serves plain HTTP on `127.0.0.1` only; a real instance lists who may
  create spaces and serves HTTPS.
- Any member of a space can forge audio that appears to come from another
  member of the same space: a known limit until group key management
  arrives ([`docs/spec.md`](docs/spec.md), section 8).

## Supported versions

Drywire is in early development and has no release yet. Fixes go to the
`main` branch.
