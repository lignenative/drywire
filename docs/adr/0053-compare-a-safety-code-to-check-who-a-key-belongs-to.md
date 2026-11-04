# 0053. Compare a safety code to check who a key belongs to

Status: Accepted · 2026-11-04 · over device keys once ADR 0055 ships

## Context

The lock closes when every voice played came from the key the server
listed for its person (ADR 0041). It says what it cannot prove: that the
server listed the right key. A dishonest instance could list a key of its
own under a member's name, decrypt what it is sent and pass it on. Only
the two people can rule that out, by comparing something both sides
compute from the keys they were given, through a channel the server does
not control: their voices, or meeting in person.

Since ADR 0051 a key is a person's, whatever their devices, so one
comparison per pair of people holds for good.

## Decision

- **A safety code per pair of people**: 20 digits in four groups of five,
  the same on both sides:

  ```text
  digest = SHA-256("drywire/safety-code/v1" ‖ lower key ‖ higher key)
  code   = (first 9 bytes of digest, big-endian) mod 10^20, in 20 digits
  ```

  The two 32-byte public keys are sorted as bytes, so the order of the
  people does not matter. Digits read aloud in any language.
- **Why 20 digits.** To sit between two people unseen, an instance must
  find keys of its own whose code with each person is the same: about
  2^66 tries for 20 digits, each one a new key pair. 10 digits (2^33)
  would fall to one computer in a day.
- **The lock's panel shows it**, for each other person, with "Mark as
  verified". The mark is kept on this device only, by public key and with
  the name it had, in the interface's own storage: it is the person's
  judgement, not something a server should hold or learn.
- **A verified person** shows apart from a merely checked one. **A
  person whose name was verified with another key** shows "new key": the
  code must be compared again.
- The code is computed by the interface itself (`desktop/src/safety.ts`,
  WebCrypto's SHA-256), the same in the app and the web client.

## Consequences

- The lock's promise gains its last link: once two people compared their
  code, a closed lock means nobody stands between them, whatever the
  server does.
- A verification lives on one device: another device of the same person
  starts unverified. Carrying it in the account's vault is possible later.
- Nothing changes on the wire or on the server.
- "New key" matches people by name, which anyone can choose: it is a
  warning to look closer, never a proof.
