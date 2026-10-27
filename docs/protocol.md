# Drywire wire protocol

Version 1 · Status: draft, implemented by the relay

This document is the normative description of the packets Drywire sends over
UDP and of the end-to-end encryption that protects audio. The Go relay
(`server/`) and the Rust core (`core/`) share no code: this document and the
binary test vectors in [`core/drywire-proto/vectors/`](../core/drywire-proto/vectors/)
are their contract. When the format changes, this document, the vectors and
both implementations change together.

The key words MUST, MUST NOT, SHOULD and MAY are used as in RFC 2119.

## 1. Conventions

- All integers are unsigned and **big-endian** (network byte order).
- `‖` means byte concatenation.
- Byte strings in examples and in the vector manifest are written in
  lowercase hexadecimal.
- Sizes are in bytes unless stated otherwise.

## 2. Transport

- Every packet is one UDP datagram. There is no fragmentation and no
  reassembly at the Drywire level.
- A datagram MUST NOT be larger than **1200 bytes**. That size fits in the
  path MTU of practically every network, including tunnels, so no datagram is
  fragmented by IP. The relay drops anything larger.
- Clients talk only to the relay. The relay copies audio packets to the
  other members of the room.

## 3. Header

Every packet starts with the same 16-byte header. It is sent in clear text so
the relay can route the packet, and it is authenticated as the additional data
of the encryption, so nobody can change it without the receiver noticing.

```text
 0               1               2               3
 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7
+---------------+---------------+---------------+---------------+
|    Version    |     Type      |     Flags     |   Key epoch   |  bytes 0–3
+---------------+---------------+---------------+---------------+
|                            Session                            |  bytes 4–7
+---------------------------------------------------------------+
|                           Sequence                            |  bytes 8–11
+---------------------------------------------------------------+
|                           Timestamp                           |  bytes 12–15
+---------------------------------------------------------------+
```

| Offset | Field | Size | Meaning |
| --- | --- | --- | --- |
| 0 | Version | 1 | Protocol version. `1` for this document. |
| 1 | Type | 1 | Packet type, see section 4. |
| 2 | Flags | 1 | Bit field, see section 5. |
| 3 | Key epoch | 1 | Which sender key encrypts an audio packet, or which epoch an `announce` is for (section 8). `0` in every other type. |
| 4 | Session | 4 | Identifies the sender to the relay. Assigned when joining a room. `0` is never a valid session. |
| 8 | Sequence | 4 | Meaning depends on the type (section 6). |
| 12 | Timestamp | 4 | Meaning depends on the type (section 6). |

Example: an audio packet of version 1, no flags, key epoch 0, session
`0x11223344`, sequence 7 and timestamp 76800 starts with:

```text
01 01 00 00  11 22 33 44  00 00 00 07  00 01 2c 00
```

**Byte 0 is the version in every version of the protocol, forever.** Any other
part of the header may change in a future version.

## 4. Packet types

| Code | Type | Sent by | Total size | Answered with |
| --- | --- | --- | --- | --- |
| `0x01` | `audio` | client | 33 to 1200 | copied to the other members |
| `0x02` | `hello` | client | exactly 56 | `hello_ack` |
| `0x03` | `hello_ack` | relay | exactly 56 | — |
| `0x04` | `keepalive` | client | exactly 16 | — |
| `0x05` | `ping` | client | exactly 32 | `pong` |
| `0x06` | `pong` | relay | exactly 32 | — |
| `0x07` | `announce` | client | exactly 112 | copied to the other members |

Code `0x00` and codes `0x08` to `0xff` are not defined in version 1. A
receiver drops a packet with an undefined type.

Control packets (every type except `audio`) have an exact size. A control
packet of any other size is invalid.

## 5. Flags

| Bit | Mask | Name | Meaning |
| --- | --- | --- | --- |
| 0 | `0x01` | `redundant` | The audio payload also carries the previous frame (section 7). |
| 1 | `0x02` | `end_of_spurt` | Last audio packet before the sender's voice gate closes. |
| 2–7 | `0xfc` | reserved | |

- Flags are only defined for `audio`. Senders MUST set the flags of every
  other type to `0`.
- Senders MUST set reserved bits to `0`. Receivers MUST ignore them.

## 6. Packets

### 6.1 `hello` — open or move a UDP session

A client sends `hello` to tell the relay "this address now speaks for my
session". The client obtained its session number and a 32-byte **session
secret** when it joined the room (from the control plane, over TLS; in phase 1
from a configuration file). The secret never travels over UDP.

```text
header (16) ‖ counter (8) ‖ mac (32)
```

| Field | Value |
| --- | --- |
| Header | `type = 0x02`, `flags = 0`, `key epoch = 0`, `session` = the client's session, `sequence = 0`, `timestamp = 0` |
| Counter | 64-bit value, strictly greater than the counter of every earlier `hello` for this session |
| MAC | `HMAC-SHA256(key = session secret, message = header ‖ counter)` |

- The counter makes every `hello` unique, so a captured `hello` cannot be
  replayed. Each attempt, including a retry after a lost `hello_ack`, uses a
  new counter. Clients SHOULD use the current Unix time in milliseconds, which
  keeps growing across restarts of the client.
- The relay accepts the `hello` if the session exists, the MAC is correct
  (compared in constant time) and the counter is greater than the last
  accepted one. It then **binds** the session to the source IP address and
  port of the datagram, replacing any earlier binding, and answers with
  `hello_ack`.
- On a network change (Wi-Fi to cellular, new NAT mapping), the client sends a
  new `hello` from its new address.
- An invalid `hello` gets no answer at all.

### 6.2 `hello_ack` — the relay confirms the binding

```text
header (16) ‖ counter (8) ‖ mac (32)
```

| Field | Value |
| --- | --- |
| Header | `type = 0x03`, `flags = 0`, `key epoch = 0`, `session` = the client's session, `sequence = 0`, `timestamp = 0` |
| Counter | Copied from the `hello` it answers |
| MAC | `HMAC-SHA256(key = session secret, message = header ‖ counter)` |

The MAC proves to the client that the answer comes from a relay that knows
its secret. Because the type byte is part of the MAC input, a `hello` can
never pass as a `hello_ack` or the other way round. A client ignores a
`hello_ack` whose counter does not match its last `hello`.

### 6.3 `keepalive` — stay bound while silent

```text
header (16)
```

`type = 0x04`, every other field `0` except `session`. No payload, no answer.
It keeps the NAT mapping open and tells the relay the client is still there.
A client sends one every **10 seconds** when it has sent nothing else in that
time (typically while its voice gate is closed and pings are off).

### 6.4 `ping` — measure the round trip

```text
header (16) ‖ client time (8) ‖ zero (8)
```

| Field | Value |
| --- | --- |
| Header | `type = 0x05`, `flags = 0`, `key epoch = 0`, `session`, `sequence` = ping number chosen by the client, `timestamp = 0` |
| Client time | Any 64-bit value; the relay copies it back unchanged. Clients SHOULD put their monotonic clock in microseconds. |
| Zero | Eight zero bytes. They make `ping` as large as `pong`, so the relay never answers with more bytes than it received. |

Clients send one `ping` per second while in a room.

### 6.5 `pong` — round trip and uplink loss

```text
header (16) ‖ client time (8) ‖ audio received (4) ‖ audio lost (4)
```

| Field | Value |
| --- | --- |
| Header | `type = 0x06`, `flags = 0`, `key epoch = 0`, `session`, `sequence` copied from the `ping`, `timestamp = 0` |
| Client time | Copied from the `ping` |
| Audio received | Audio packets the relay received from this session since its last accepted `hello`, duplicates excluded |
| Audio lost | Audio packets the relay expected from this session in the same period but never received (section 10.4) |

The client subtracts the client time from its clock to get the round-trip
time, and compares successive counters to see its uplink loss.

### 6.6 `audio` — one encrypted Opus frame

```text
header (16) ‖ ciphertext (n) ‖ tag (16)
```

| Field | Value |
| --- | --- |
| Header | `type = 0x01`, flags as in section 5, `key epoch` (section 8), `session`, `sequence`, `timestamp` |
| Sequence | Counter of audio packets within the key epoch: `0` for the first audio packet of an epoch, then `+1` for each audio packet sent. It does not advance while the gate is closed. |
| Timestamp | Position of the first sample of the primary frame, in 48 kHz samples, modulo 2³². It starts at any value and keeps advancing while the gate is closed, so the receiver knows how long the silence was. |
| Ciphertext and tag | The audio payload (section 7), encrypted with AES-256-GCM (section 8.4) |

The plaintext is at least one byte, so an audio packet is at least
`16 + 1 + 16 = 33` bytes.

### 6.7 `announce` — publish the salt of a key epoch

```text
header (16) ‖ sender salt (32) ‖ signature (64)
```

| Field | Value |
| --- | --- |
| Header | `type = 0x07`, `flags = 0`, `key epoch` = the epoch announced, `session` = the sender's session, `sequence = 0`, `timestamp = 0` |
| Sender salt | The `sender_salt` of that epoch (section 8.1) |
| Signature | Ed25519 signature by the sender's identity key over the announcement message of section 8.3 |

The relay copies it to the other members of the room like an audio packet
(section 10.1); receivers use it to derive the sender's key (section 8.3).

## 7. Audio payload

The plaintext inside an audio packet holds one Opus frame, the **primary
frame**, and optionally a copy of the previous one, the **redundant frame**.

Without the `redundant` flag:

```text
primary frame (≥ 1)
```

With the `redundant` flag:

```text
redundant length (2) ‖ redundant frame (redundant length) ‖ primary frame (≥ 1)
```

- Opus frames describe their own duration in their first byte (the TOC byte,
  RFC 6716 section 3.1), so no duration field is needed.
- The redundant frame is the primary frame of the audio packet with
  `sequence − 1` in the same key epoch. Its timestamp is the primary
  timestamp minus its own duration. A sender sets the flag only if it sent
  that previous packet, so never on the first packet of a talk spurt.
- `redundant length` MUST be at least 1, and the primary frame MUST be at
  least one byte. A payload that breaks either rule, or whose redundant length
  goes past the end of the plaintext, is invalid (`bad_framing`).
- Each frame is at most 575 bytes, so that a primary and a redundant frame
  always fit in one datagram (`16 + 2 + 2 × 575 + 16 = 1184` bytes). A
  frame over 575 bytes is invalid (`bad_framing`). A 10 ms frame at 64 kb/s
  is about 80 bytes; receivers rely on the bound to keep frames in fixed
  buffers.
- Redundancy only helps if the receiver's jitter buffer holds at least two
  frames, because the copy of frame *n* arrives with packet *n + 1*.

## 8. End-to-end encryption

The relay never holds a key that decrypts audio. Keys come from a secret
shared by the members of a space.

### 8.1 Inputs

| Name | Size | Origin |
| --- | --- | --- |
| `space_key` | 32 | 256 random bits, shared through the invitation link |
| `room_id` | 16 | Random identifier of the room, chosen when the room is created |
| `sender_public_key` | 32 | The sender's Ed25519 public key (RFC 8032) |
| `sender_salt` | 32 | 256 random bits drawn by the sender for each key epoch |

### 8.2 Key derivation

Both steps use HKDF-SHA256 (RFC 5869) with a 32-byte output. An empty salt
means HKDF's default salt (32 zero bytes).

```text
room_key   = HKDF-SHA256(ikm  = space_key,
                         salt = empty,
                         info = "drywire/room/v1" ‖ room_id)

sender_key = HKDF-SHA256(ikm  = room_key,
                         salt = sender_salt,
                         info = "drywire/sender/v1" ‖ sender_public_key)
```

The `info` strings are ASCII, without a terminating zero.

Why this design: AES-GCM breaks completely if a key and nonce pair is ever
used twice. Each `sender_key` depends on randomness its sender drew, so two
senders never share a key, a restarted client never reuses an old one, and the
relay cannot force a collision by handing out the same session number twice.

### 8.3 Key epochs and the sender announcement

A **key epoch** is the period during which a sender uses one `sender_salt`.
The header's key epoch byte tells receivers which key to use.

- A sender starts at epoch `0` when it joins a room, with a fresh salt.
- To change keys it draws a new salt, increments the epoch by one (modulo
  256), and restarts its audio sequence at `0`.
- A sender MUST start a new epoch before its audio sequence reaches 2³¹
  (`0x80000000`). At 400 packets per second that is after about 62 days, so
  in practice an epoch lasts as long as the sender stays in the room.
- The sender publishes each epoch's salt in a **sender announcement**, an
  `announce` packet (section 6.7) that the relay copies to the other members
  of the room. It carries `sender_salt` and the sender's Ed25519 signature
  over the message below; `session` and `epoch` come from its header, and
  `room_id` is known to every member.

```text
message = "drywire/sender-announce/v1" ‖ room_id (16) ‖ session (4)
          ‖ epoch (1) ‖ sender_salt (32)
```

- UDP may lose an announcement, so the sender sends it when the epoch
  starts, again immediately before the first audio packet of every talk
  spurt, and then once per second while its gate stays open. It sends the
  first one of an epoch before any audio of that epoch.
- A receiver verifies the signature with the public key it knows for that
  session (from the control plane; in phase 1 from a configuration file)
  before deriving the key. An announcement from a session with no known
  public key, or whose signature does not verify, is dropped
  (`bad_signature`).
- A new announcement for the same `(session, epoch)` with a different salt
  replaces the old one: the sender restarted. Receivers keep the keys of at
  least the current and the previous epoch of each sender, so packets
  reordered across an epoch change still decrypt.
- An audio packet for a `(session, epoch)` with no key yet cannot be
  decrypted. Receivers drop it and count it as `no_key`; they do not hold it
  back, since holding audio would add latency and the next announcement is
  at most one second away.

### 8.4 Encryption of an audio packet

```text
nonce      = 0x00 × 8 ‖ sequence (4)                 12 bytes
ciphertext ‖ tag = AES-256-GCM-Seal(key       = sender_key,
                                    nonce     = nonce,
                                    plaintext = audio payload,
                                    aad       = header (16))
```

- The nonce is the audio sequence number, zero-padded on the left to 96 bits.
  Within one epoch a sender never reuses a sequence number, so it never
  reuses a nonce with the same key.
- The whole header is the additional authenticated data: changing the
  session, the epoch, the flags or the timestamp makes decryption fail.
- A receiver that fails to decrypt a packet drops it (`bad_tag`) and counts
  it.

### 8.5 Replay window

Each receiver keeps one replay window per `(sender session, key epoch)` and
drops packets it has already seen. The window remembers the highest sequence
accepted so far and which of the 64 sequences ending at that one were seen.

For an incoming `sequence`, after successful decryption:

| Case | Verdict | Effect |
| --- | --- | --- |
| First packet of this window | `new` | `highest = sequence`, mark it seen |
| `sequence > highest` | `new` | Slide the window forward to `sequence`, mark it seen |
| `highest − 63 ≤ sequence ≤ highest`, not seen | `new` | Mark it seen |
| `highest − 63 ≤ sequence ≤ highest`, already seen | `duplicate` | Drop |
| `sequence < highest − 63` | `too_old` | Drop |

A receiver checks the window **after** decryption, so forged packets cannot
move it. The relay, which cannot decrypt, uses the same window only to drop
duplicates and to count losses (section 10.4).

## 9. Errors

These codes name why a packet is invalid. Implementations log or count them;
they are never sent on the wire. The test vectors use the same codes.

| Code | Detected by | Condition |
| --- | --- | --- |
| `too_short` | anyone | Fewer than 16 bytes |
| `too_large` | anyone | More than 1200 bytes |
| `unsupported_version` | anyone | Version byte not supported by this implementation |
| `unknown_type` | anyone | Type not defined in this version |
| `bad_length` | anyone | Control packet of the wrong size, or audio packet under 33 bytes |
| `zero_session` | anyone | Session is `0` |
| `bad_mac` | relay, client | MAC of a `hello` or `hello_ack` does not verify |
| `bad_tag` | receiver | AES-GCM decryption fails |
| `bad_framing` | receiver | Audio payload breaks the rules of section 7 |
| `bad_signature` | receiver | Signature of an `announce` does not verify, or its session has no known public key |

Checks run in the order of this table: a 10-byte datagram is `too_short` even
though its version byte is also wrong.

## 10. Relay behaviour

### 10.1 Processing a datagram

1. Apply the per-IP rate limit. Drop if exceeded.
2. Parse the header and check the size (section 9, first six codes). Drop if
   invalid.
3. If the type is `hello`, verify it (section 6.1). If valid, bind the session
   and send `hello_ack`. Stop.
4. Otherwise the session MUST exist and be bound to exactly the source address
   of the datagram. Drop if not.
5. Apply the per-session rate limits. Drop if exceeded.
6. By type:
   - `keepalive`: nothing more.
   - `ping`: send `pong` to the bound address.
   - `audio`: count it (section 10.4), drop it if it is a duplicate, else send
     the unchanged datagram to the bound address of every other bound session
     in the same room.
   - `announce`: send the unchanged datagram to the same addresses as audio.
     It is not counted as audio, and the relay does not verify its signature,
     which needs the members' public keys.
   - `hello_ack` and `pong` come only from the relay: drop.

Every accepted datagram refreshes the session's last-seen time.

### 10.2 No amplification

- The relay sends nothing to an address that has not proven, with a valid
  `hello`, that it holds the session secret.
- A reply (`hello_ack`, `pong`) is never larger than the packet that caused
  it: both are exactly the size of their request.
- Audio and announcements go only to bound addresses of the same room, so a
  forged source address cannot turn the relay against a third party.

### 10.3 Limits and timeouts

These are the values of the reference relay for version 1. Clients MUST stay
well below them.

| Limit | Value | Why |
| --- | --- | --- |
| Packets per session | 600 per second, bursts of 200 | 400 packets per second at 2.5 ms frames, plus pings |
| Bytes per session | 128 KiB per second, bursts of 32 KiB | High-bitrate audio with redundancy fits several times over |
| Packets per source IP | 3000 per second, bursts of 600 | A few clients behind the same NAT |
| Session timeout | 30 seconds without a packet | Three missed keepalives; the binding is then forgotten |

### 10.4 Uplink loss counting

The relay tracks, per session, the key epoch of the last audio packet, a
replay window (section 8.5), and two counters reset by every accepted
`hello`:

- An audio packet of the tracked epoch goes through the replay window.
  `duplicate` is dropped and not counted. `new` and `too_old` are counted as
  received.
- An audio packet of epoch `tracked + 1` (modulo 256), or the first audio
  packet after a `hello`, starts a new tracked epoch with a fresh window.
- An audio packet of any other epoch is forwarded but not counted.
- `lost` is the number of sequences the relay expected, from the lowest to
  the highest sequence received in each tracked epoch, minus the packets it
  received. Late packets lower it again. It never goes below zero.

## 11. Versioning

- The version byte is incremented for every **breaking change**: a change that
  an implementation of the previous version would misread. Examples: a new
  header layout, a new payload layout, a new flag that changes the payload
  layout, a change in key derivation.
- Adding a packet type is not breaking, because receivers drop types they do
  not know. Adding a flag that receivers may safely ignore is not breaking.
- The relay accepts the **current and the previous** version. It forwards
  packets unchanged and never translates between versions.
- A client sends with the highest version that both it and the relay support,
  and decodes the current and the previous version.
- The relay never answers a packet of an unsupported version, because it
  cannot verify it. Clients learn which versions the relay supports from its
  HTTP API: `GET /v1/version` returns them as `protocol_versions`. A client
  whose version is not in that list tells its user to update.

## 12. Test vectors

The directory `core/drywire-proto/vectors/` holds one `.bin` file per packet
and a `manifest.json` describing them. The Go and the Rust test suites both
read it. The vectors are regenerated with `make vectors`, and a Go test fails
if the files on disk differ from what the generator produces.

`manifest.json` contains:

- `protocol_version`: the version these vectors describe.
- `inputs`: every key and secret used, in hex: `session_secret`,
  `space_key`, `room_id`, `sender_seed` (the 32-byte Ed25519 seed),
  `sender_public_key`, and the derived `room_key`.
- `epochs`: for each key epoch used, its `sender_salt`, the derived
  `sender_key`, and the announcement `signature`.
- `vectors`: one entry per file with `name`, `file`, `description`, and
  either `error` (one of the codes of section 9, for an invalid packet) or the
  expected decoded fields: `header`, plus `hello`, `ping`, `pong`, `audio` or
  `announce` depending on the type.
- `replay_window`: a sequence of steps (`sequence`, expected `verdict`)
  applied in order to one fresh window.
- `login`: a control plane login by the sender's identity (section 13.2):
  `audience`, `challenge`, the signed `message` and its `signature`.
- `invitation`: an invitation link (section 14) to the space of
  `inputs.space_key`: `instance`, `code`, the decoded `payload` and the
  `link` itself.

The Opus frames in the vectors are arbitrary bytes, not real audio: the
protocol carries frames without looking inside them.

## 13. Control plane

The control plane is the server's HTTP API. It hands out identities'
session tokens, spaces, rooms and invitation codes, and the UDP session
(number and secret) a client needs before its first `hello`. It never sees
a space key and never decrypts audio.

### 13.1 Transport and conventions

- HTTPS with TLS 1.3, from the server itself or from a reverse proxy in
  front of it. The server refuses to listen in plain HTTP on anything but a
  loopback address, and clients refuse plain HTTP to anything but a
  loopback address.
- Requests and responses are JSON (`Content-Type: application/json`), at
  most 64 KiB per request. Keys, ids, codes and signatures are lowercase
  hex. Times are RFC 3339 strings in UTC.
- Changes are additive: a server ignores request fields it does not know,
  and a client ignores response fields it does not know (section 11).
- Errors have a 4xx or 5xx status and a body `{"error": "<message>"}`, in
  English, safe to show to the user.
- Nothing secret ever goes in a URL: tokens travel in the `Authorization`
  header, codes and keys in request bodies.

### 13.2 Identity and login

An identity is an Ed25519 key pair generated by the client on its first
start (spec section 8). There is no account and no password: a client
proves that it holds the private key.

1. `POST /v1/auth/challenge` with an empty body. The answer is
   `{"challenge": "<32 bytes>", "expires_in": 60}`.
2. The client signs the **login message** with its identity key:

   ```text
   message = "drywire/login/v1" ‖ challenge (32) ‖ audience
   ```

   The **audience** is the instance's canonical origin (section 13.3) as
   the client reached it, in ASCII. It binds the signature to one instance:
   a malicious instance that passes on another instance's challenge gets a
   signature the other instance refuses.
3. `POST /v1/auth/login` with
   `{"public_key": "<32 bytes>", "challenge": "<32 bytes>", "signature": "<64 bytes>"}`.
   A valid signature over a challenge the server issued less than 60
   seconds ago gives `{"token": "<opaque>", "expires_in": 3600}`.

- Each challenge answers one login attempt, valid or not. Every failure is
  the same `401`, which says nothing about why.
- The token goes in every other request as `Authorization: Bearer <token>`.
  It is valid for one hour; on a `401` the client logs in again. Tokens are
  kept in the server's memory only, so a restart logs everyone out.
- Logging in records the time on the identity: identities that have not
  logged in for 12 months are purged (spec section 9).
- A key the server does not know yet can log in. Its identity, with a
  display name, is created when it first creates a space or redeems an
  invitation; until then the server stores nothing about it.

### 13.3 Instance address

An instance is named by its **canonical origin**: the scheme (`https`, or
`http` for a loopback development server), `://`, the host in lowercase
(an IPv6 address in brackets), and `:port` unless the port is the scheme's
default. Nothing else: no user name, path, query or fragment.
`https://Voice.Example.org:443/` becomes `https://voice.example.org`.
The server is configured with its own (`DRYWIRE_PUBLIC_URL`).

### 13.4 Spaces, rooms and members

A space has exactly one **owner**, any number of **admins** and
**members** (spec section 8). A member joins rooms and talks; an admin also
creates and deletes rooms, creates and revokes invitations, removes and
bans members; the owner also changes roles, hands the space over and
deletes it. Someone may only remove or ban a member of a lower role. A
space the caller is not a member of answers `404`, as if it did not exist.

| Request | Who | Body → answer |
| --- | --- | --- |
| `GET /v1/spaces` | anyone logged in | → `[{space, role}]`, the caller's spaces |
| `POST /v1/spaces` | the instance's space creators | `{name, room?, display_name}` → `201` space view |
| `GET /v1/spaces/{space}` | member | → space view |
| `DELETE /v1/spaces/{space}` | owner | → `204` |
| `POST /v1/spaces/{space}/rooms` | admin | `{name}` → `201` room |
| `DELETE /v1/rooms/{room}` | admin | → `204` |
| `PATCH /v1/spaces/{space}/members/{public_key}` | owner | `{role}` → `204`; `owner` hands the space over, the old owner becomes admin |
| `DELETE /v1/spaces/{space}/members/{public_key}` | the member themself, or a higher role | → `204`; the owner cannot leave |
| `GET /v1/spaces/{space}/bans` | admin | → `[public_key]` |
| `POST /v1/spaces/{space}/bans` | admin | `{public_key}` → `204`, removes a member at once |
| `DELETE /v1/spaces/{space}/bans/{public_key}` | admin | → `204` |

- A **space view** is `{space: {id, name, max_members, created_at}, role,
  rooms: [{id, space_id, name}], members: [{public_key, name, role,
  joined_at}]}`. A room's `id` is its `room_id` (section 8.1).
- Names (display, space, room) are 1 to 32 characters of valid UTF-8,
  trimmed, without control or format characters.
- Who may create spaces is the operator's choice
  (`DRYWIRE_SPACE_CREATORS`): an instance is not an open sign-up.
- A space has a maximum number of members, set when it is created
  (`DRYWIRE_MAX_MEMBERS`, 50 by default).
- A ban names a public key and outlives the identity, so deleting an
  identity and coming back with the same key does not lift it.

### 13.5 Invitations

| Request | Who | Body → answer |
| --- | --- | --- |
| `POST /v1/spaces/{space}/invitations` | admin | `{expires_in?, max_uses?}` → `201` `{invitation, code, instance}` |
| `GET /v1/spaces/{space}/invitations` | admin | → the invitations that can still be used, without codes |
| `DELETE /v1/spaces/{space}/invitations/{id}` | admin | → `204` |
| `POST /v1/invitations/redeem` | anyone logged in | `{code, display_name}` → space view |

- An invitation lasts from one minute to 30 days (`expires_in` in seconds,
  7 days by default) for 1 to 100 uses (10 by default).
- The `code` is returned once, at creation, and the server keeps only its
  SHA-256. The client builds the link (section 14) from `instance`, the
  code and the space key it holds.
- Redeeming creates the caller's identity with `display_name` if it is
  new, and makes it a member. A member who redeems again gets the space
  without using up the code. An unknown, expired or used-up code all
  answer the same `404`; a banned key gets `403`; a full space `409`.

## 14. Invitation links

An invitation carries everything a new member needs: where the instance
is, a code the instance checks, and the space key, which the instance never
sees.

```text
https://drywire.gg/i#<base64url(payload)>

payload = version (1) ‖ code (16) ‖ space_key (32) ‖ instance
```

| Field | Value |
| --- | --- |
| Version | `1` |
| Code | 16 random bytes from the instance (section 13) |
| Space key | The space's `space_key` (section 8.1) |
| Instance | The instance's canonical origin (section 13.3), UTF-8, to the end of the payload |

- Base64url is RFC 4648 section 5 without padding. Decoders refuse padding,
  other alphabets and non-zero spare bits, so a payload has one spelling.
- **Everything is in the fragment**, which browsers never send to a server:
  the code and the key appear in no access log and no `Referer` header. The
  page at `https://drywire.gg/i` is static and makes no network request
  with the fragment.
- The link points to the project's domain even for a self-hosted instance,
  because universal links and App Links only work for domains the apps
  declare. Desktop apps also accept `drywire://i#<payload>`.
- A client refuses a payload of another version (the user needs an update)
  and an instance that is not in canonical form.
- Codes expire and have a maximum number of uses. The instance stores only
  their SHA-256.
