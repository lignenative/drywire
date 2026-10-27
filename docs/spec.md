# Drywire — Specification

Draft 2 · 2026-10-27 · Publisher: Ligne Native · License: MIT

Drywire is an open source team voice chat built for the lowest possible
mouth-to-ear latency. This document is the reference for what we build and
why. When the code and this document disagree, the code wins and this
document gets fixed.

## 1. Vision and principles

- **As low as physics allows.** The goal is not a fixed number: it is the
  lowest median latency we can reach, release after release. Every figure in
  this document is a hypothesis until a measurement replaces it.
- **Measured, never promised.** Any latency shown in the app or announced in
  public comes from a recorded measurement. We never say "zero latency".
- **Untouched voice.** No noise suppression, no automatic gain control, no
  echo cancellation. A voice gate decides *when* audio is sent; it never
  changes the signal itself.
- **Voice only.** No text chat, no screen sharing, no video.
- **Small and durable.** The server is a single Go binary, standard library
  first. Few dependencies, all under permissive licenses.
- **Native and polished.** Each platform gets a native app. Every screen has
  to justify its existence.
- **Easy to read.** The codebase is written so that a new developer can
  understand it and contribute within a day.

## 2. Primary use case

A three-player competitive team. Two players are on PC in the same city. The
third plays on a console, which cannot run third-party apps, so they use their
Android phone as a voice client: one wired earbud with an inline microphone,
worn under the console headset. The relay runs on a small VPS in a datacenter
a few hundred kilometres away.

What the team observed with existing tools:

- Voice processing in mainstream apps clips the start of sentences and adds
  latency.
- The console player's current chain (console voice app, headset on the
  controller) adds around half a second to one second in each direction.
- A well-known self-hosted voice server with processing disabled feels almost
  instant between two PCs, but has not been measured yet. It is the baseline
  to beat.
- Bluetooth is ruled out for low latency: on its own it exceeds our whole
  budget.

The project is built for this team first, and designed from day one to serve
any group, on every platform.

## 3. Glossary

| Term | Meaning |
| --- | --- |
| Mouth-to-ear latency | Time between a sound reaching the sender's microphone and leaving the receiver's earphone. |
| Frame | A fixed slice of audio (2.5, 5 or 10 ms) encoded as one unit by Opus. |
| CELT | The low-delay mode of the Opus codec. We use it exclusively. |
| Talk spurt | A continuous stretch of speech between two silences. |
| Voice gate | Sends frames only while the input level is above a threshold. |
| Jitter | Variation in packet arrival times. |
| Jitter buffer | A small queue on the receiver that absorbs jitter before playback. |
| PLC | Packet loss concealment: Opus synthesises a plausible frame when one is missing. |
| Relay | The server process that forwards encrypted audio packets between members. |
| Data plane | The UDP traffic carrying audio. |
| Control plane | The HTTPS API handling identities, spaces, rooms and presence. |

## 4. Latency budget

These figures are hypotheses to be replaced by phase 0 and phase 1
measurements. Frame size is the main lever we control, so the budget is shown
for the three frame sizes CELT supports in our configuration.

| Stage (mouth → ear) | 10 ms frames | 5 ms frames | 2.5 ms frames | Lever |
| --- | --- | --- | --- | --- |
| Capture buffer | 3–10 | 3–10 | 3–10 | OS low-latency mode, smallest period the device accepts |
| Opus framing + lookahead | 12.5 | 7.5 | 5 | Frame size |
| Network, sender → relay | 4–10 | 4–10 | 4–10 | Relay location |
| Relay | < 0.1 | < 0.1 | < 0.1 | Blind forwarding, no decoding |
| Network, relay → receiver | 4–10 | 4–10 | 4–10 | Relay location |
| Jitter buffer (floor = one frame) | 10–20 | 5–20 | 2.5–20 | Frame size, network quality |
| Playback buffer | 3–10 | 3–10 | 3–10 | Same as capture |
| **Theoretical floor** | **~37** | **~27** | **~22** | |

Not in the table, and to be measured per device:

- **Wireless headsets** with a proprietary 2.4 GHz dongle add latency that
  manufacturers do not publish. The in-app loopback test measures it.
- **USB-C audio adapters** on phones may add latency compared to a 3.5 mm
  jack.

Smaller frames cost bandwidth. Each packet carries about 60 bytes of overhead
(IPv4 + UDP 28, Drywire header 16, authentication tag 16):

| Frame size | Packets per second | Overhead | Total upstream at 64 kb/s Opus |
| --- | --- | --- | --- |
| 10 ms | 100 | ~48 kb/s | ~112 kb/s |
| 5 ms | 200 | ~96 kb/s | ~160 kb/s |
| 2.5 ms | 400 | ~192 kb/s | ~256 kb/s |

This is negligible on a wired connection. Phase 1 measures all three sizes and
picks the default from the data; the setting stays available per client.

**Targets.** There is no fixed target. Each phase must beat the baseline
measured in phase 0 on the same setup, and every later release must not
regress. A regression in measured latency is treated as a bug.

## 5. Scope and platforms

Platforms, in order of priority:

1. **Windows** — the team's PC players.
2. **iOS and macOS** — the maintainer's own devices, for daily testing.
3. **Android** — the console player's voice client.
4. **Linux** — desktop, same app as Windows.
5. **Web** — a fallback client, never as fast as a native one.

Out of scope for every version: text chat, screen sharing, video, recording,
bots, AI noise suppression, console apps (consoles do not allow them).
Bluetooth devices work, but the app shows a latency warning.

Parked until real-world tests ask for them:

- **Open-mic mode** (continuous transmission, no gate). It removes the
  silences the jitter buffer uses to resynchronise, so it would need adaptive
  resampling. We only build it if the gate turns out to hurt in practice.
- **Controller mute button** (for example reading the DualSense microphone
  button over HID and syncing its LED). Nice to have; keyboard shortcut and
  tray icon come first.

## 6. Architecture

```text
 ┌──────────────┐ ┌──────────────┐ ┌──────────────┐ ┌──────────────┐
 │ Desktop app  │ │  Apple apps  │ │ Android app  │ │  Web client  │
 │ Tauri, React │ │   SwiftUI    │ │   Compose    │ │ React, WASM  │
 └──────┬───────┘ └──────┬───────┘ └──────┬───────┘ └──────┬───────┘
        └────────────────┴───────┬────────┴────────────────┘
                     ┌───────────┴───────────┐
                     │ drywire-core (Rust)   │  audio I/O, Opus, jitter
                     │ same code everywhere  │  buffer, protocol, crypto
                     └───────────┬───────────┘
              UDP (encrypted) ── │ ── HTTPS (control plane)
                     ┌───────────┴───────────┐
                     │ drywire-server (Go)   │  blind relay + control
                     └───────────────────────┘  plane + SQLite
```

Every client embeds the same Rust core. Only the user interface changes from
one platform to another. The server forwards encrypted UDP packets without
reading them, and manages identities, spaces and rooms over HTTPS.

### Server: `drywire-server` (Go)

- A single static binary (`CGO_ENABLED=0`), run in a `scratch` container or
  as a systemd service.
- **Data plane**: one UDP socket, blind relay. It receives a packet and sends
  it to the other members of the room. Active rooms live in memory.
- **Control plane**: HTTP API from the standard library. JSON for commands,
  Server-Sent Events for live presence (WebSocket is not in the standard
  library). TLS is either terminated by a reverse proxy in front of it, or
  handled by the server itself from certificate files.
- **Storage**: SQLite through `modernc.org/sqlite` (pure Go, no cgo) for
  identities, spaces, rooms, members and public keys.
- **Accepted dependencies**: `modernc.org/sqlite` now, `quic-go` for the web
  client later. Anything else needs an explicit decision recorded in an ADR.

### Core: `drywire-core` (Rust)

- Audio capture and playback, Opus encoding and decoding (libopus), voice
  gate, jitter buffer, protocol, encryption, statistics.
- Speaking detection, for display only.
- Exposed to Swift and Kotlin through UniFFI, used directly by Tauri, compiled
  to WASM for the web.
- **Audio I/O sits behind one small trait, with one backend per OS**, written
  directly against the OS API. Generic audio libraries do not expose the
  lowest-latency modes we need:

| OS | Backend | Mode |
| --- | --- | --- |
| Windows | WASAPI | Exclusive mode for capture when the microphone is not shared; shared mode with the smallest `IAudioClient3` period for playback, because game audio uses the same headset |
| iOS, macOS | Core Audio | No voice processing; smallest I/O buffer the hardware accepts (the app configures `AVAudioSession` on iOS) |
| Android | AAudio | Exclusive sharing mode, low-latency performance mode |
| Linux | PipeWire | ALSA as fallback |
| Web | AudioWorklet | — |

Why Rust: real-time audio callbacks without a garbage collector, and one code
base for every platform.

### Clients

| Platform | Stack |
| --- | --- |
| Windows, Linux | Tauri v2 + React (one desktop app) |
| iOS, macOS | SwiftUI multiplatform |
| Android | Kotlin + Jetpack Compose |
| Web | React + WASM core, WebTransport datagrams |

JavaScript tooling is Bun: package manager, script runner and test runner.
Vite builds and serves the React front ends. Node.js is not required.

The server (Go) and the core (Rust) share no code. Their contract is
`docs/protocol.md`, checked by binary test vectors that both test suites read.

## 7. Network and audio protocol

Raw UDP, a 16-byte header, Opus CELT frames, end-to-end encryption. This
section is the starting point; `docs/protocol.md` is the normative version.

### Audio

- Opus, 48 kHz, mono, `OPUS_APPLICATION_RESTRICTED_LOWDELAY` (CELT only).
- Frame size 2.5, 5 or 10 ms; default chosen in phase 1 from measurements.
- Bitrate 48 to 64 kb/s, tuned by listening tests. Smaller frames need a
  higher bitrate for the same quality.
- **Voice gate, on by default.** It opens on the first frame above the
  threshold, with no lookahead, so it adds no latency. It stays open for
  about 150 ms after the level drops (hold), and closes at a lower threshold
  than it opens (hysteresis). It only decides whether frames are sent; it
  never alters samples. The trade-off: a very soft onset below the threshold
  is not sent, which is why the threshold is calibrated per user.
- **Redundancy, off by default.** When enabled, each packet also carries the
  previous frame. It only helps if the receiver's jitter buffer is at least
  two frames deep: the copy of frame *n* arrives with packet *n + 1*, one
  frame later. It is turned on when losses are observed, and accepted as
  costing one frame of latency. Otherwise, a missing frame is covered by PLC.

### Header (16 bytes, authenticated, not encrypted)

| Field | Size | Purpose |
| --- | --- | --- |
| Version | 1 byte | Protocol version |
| Type | 1 byte | `audio`, `hello`, `hello_ack`, `keepalive`, `ping`, `pong`, `announce` |
| Flags | 1 byte | Redundant frame present, last frame of a talk spurt |
| Key epoch | 1 byte | Which sender key encrypts this packet (see section 8) |
| Session | 4 bytes | Assigned by the server when joining a room; identifies the sender to the relay |
| Sequence | 4 bytes | Per-sender counter |
| Timestamp | 4 bytes | In 48 kHz samples, wraps after about 24 hours |

**Payload**: the Opus frame (and the redundant frame, if any), encrypted with
AES-256-GCM. The header is the additional authenticated data. The tag is 16
bytes. Keys and nonces are described in section 8.

### Relay

- The client gets a session id and a 32-byte session secret from the control
  plane, over TLS.
- It opens its UDP session with a `hello` carrying a counter and an HMAC of
  the header and counter, keyed with the session secret. The token never
  travels in clear text, and a captured `hello` cannot be replayed.
- The server then binds the session to the sender's IP address and port. On a
  network change (Wi-Fi to cellular), the client sends a new `hello` with a
  higher counter.
- The server decrypts nothing. It checks the session and the address, limits
  the rate per session and per IP, then sends the packet to the other sessions
  in the room.
- **No amplification**: the server sends nothing to an address before a valid
  `hello`, and never answers with a packet larger than the request.
- `ping` / `pong` with the server every second, to show the round-trip time.
  The `pong` also reports the uplink loss the relay saw (it can count gaps in
  sequence numbers). `keepalive` every 10 seconds while the gate is closed.

### Reception

- One jitter buffer per sender. Target depth = p95 of recent jitter plus a
  margin, never below one frame.
- **The buffer adjusts its depth only between talk spurts.** The gate's
  silences are where it resynchronises, which also absorbs the clock drift
  between the sender's microphone and the receiver's sound card (each has its
  own crystal, a few tens of ppm apart).
- Within a talk spurt: if the buffer runs dry, PLC fills the gap; if it grows
  more than two frames above target, one frame is dropped. Both should be
  rare, and both are counted in the statistics.
- Mixing on the client: a plain sum, then soft clipping to avoid saturation.
  Nothing ever touches the outgoing voice.

## 8. Identity, spaces and security

No account and no password: one key pair per device, invitations by link,
and an encryption key the server never sees.

### Identity

- An Ed25519 key pair is generated on first launch. The private key is kept
  in the OS secure storage: Keychain on Apple platforms, Keystore-backed
  encryption on Android, DPAPI on Windows, Secret Service on Linux,
  a non-extractable WebCrypto key in the browser.
- The user picks a display name. The server associates it with the public
  key.
- The client authenticates to the control plane with a signed challenge, then
  receives a short-lived session token.

### Data model

- Instance (one `drywire-server`) › Spaces (a group, like a community
  server) › Rooms › Members.
- Roles: owner, admin, member.
- A client can connect to several instances.

### Invitations

An invitation link looks like this:

```text
https://drywire.gg/i#<payload>
```

- The payload holds the instance address, the invitation code and the space
  key, base64url-encoded. **Everything is in the fragment**, which browsers
  never send to a server: the code and the key never appear in an access log
  or a `Referer` header.
- The link always points to the project's domain, even for a self-hosted
  instance, because universal links (Apple) and App Links (Android) only work
  for domains the app declares. Desktop apps register the `drywire://`
  scheme as well.
- `/i` is a static page from `site/`. It makes no network request with the
  fragment. If the app is not installed, it explains how to get it.
- Invitation codes expire and have a maximum number of uses.

### End-to-end encryption

Keys are derived with HKDF-SHA256:

```text
space_key   256 random bits, shared through the invitation link
room_key    = HKDF(space_key, info = "drywire/room/v1" ‖ room_id)
sender_key  = HKDF(room_key,  salt = sender_salt,
                              info = "drywire/sender/v1" ‖ sender_public_key)
```

- Each sender draws a fresh random 32-byte `sender_salt` every time it joins a
  room, and whenever it starts a new key epoch.
- It signs `(room_id, session, epoch, sender_salt)` with its identity key and
  sends it in an `announce` packet that the relay copies to the room
  (ADR 0007). Other members verify the signature with the public key the
  control plane gave them before using the key.
- The nonce is the 32-bit sequence number, zero-padded to 96 bits. A sender
  starts a new epoch, with a new salt, long before the sequence number wraps.
- **Why this design**: AES-GCM breaks completely if a key and nonce pair is
  ever reused. Because each key depends on randomness the sender chose, two
  senders never share a key, a restarted client never reuses an old one, and
  the server cannot force a collision by handing out the same session id
  twice.
- Each receiver keeps a replay window per sender and drops duplicates.
- **Known limit**: any member of a space holds the room key, so a member
  could forge audio that appears to come from another member. Acceptable for
  small trusted groups; group key management (MLS) addresses it later.
- Removing a member: regenerate the invitation, which creates a new space key.
  Automatic rotation comes with MLS.

### Transport security and abuse

- Control plane over TLS 1.3. Data plane encrypted end to end.
- Rate limiting per session and per IP, maximum members per space, bans by
  public key.

## 9. Personal data (GDPR)

Drywire is designed to comply with the GDPR from the start (article 25). The
operator of an instance is the data controller: the person self-hosting it,
or Ligne Native for the hosted instance.

### What is processed

| Data | Purpose | Where | Retention |
| --- | --- | --- | --- |
| Display name, public key, memberships, roles | Identify members of a space | Server, SQLite | Until the identity is deleted; identities inactive for 12 months are purged |
| IP address and port | Route audio packets | Relay memory | Duration of the session only |
| Presence (who is in which room) | Show who is online | Server memory | Duration of the session only, no history |
| Access logs | Security, incident detection | Server | 1 year, minimised: no query string, headers or `Referer`, sensitive paths masked, full IP kept |
| Backups | Recovery | Encrypted, off the server | 6 months at most |
| Push tokens (later) | Notifications when the app is closed | Server | Until the device unregisters or the identity is deleted |
| Local statistics | Diagnose audio quality | Device only | Never sent without consent |
| Audio | — | Never stored | The server cannot decrypt it |

Legal basis: providing the service the user asked for (art. 6(1)(b)); access
logs rely on legitimate interest in security (art. 6(1)(f)).

### Rules

- **Rights in the app**: export of identity data as JSON, rename, delete the
  identity (public key, memberships; owned spaces are transferred or deleted),
  leave a space. Objection and restriction are handled by contacting the
  operator, within one month.
- **A deletion survives a restore.** Deletions are written to a ledger kept
  outside the backups, and replayed after every restore.
- **No secret or content in a URL** (see invitations above). Application logs
  never contain user content or tokens.
- **Cookies**: only strictly necessary ones. No analytics without consent,
  following CNIL rules.
- **Processors** are listed in the privacy policy: the hosting provider (EU),
  and later Apple Push Notification service and Firebase Cloud Messaging,
  which involve transfers outside the EU.
- **Privacy policy** published on the project site before the first app store
  release (app stores require one).
- **Minors**: in France, users under 15 need parental consent. Competitive
  gaming reaches many teenagers, so the hosted instance must handle this, and
  a data protection impact assessment must be evaluated, before it opens to
  the public. Until then there is no public sign-up.
- **Breaches**: logs are timestamped so that an incident can be detected and
  dated; the operator notifies the CNIL within 72 hours when required.
- A template record of processing activities lives in `docs/privacy.md`.

## 10. Clients and UX

A compact window that stays out of the way during a match, untouched voice,
and honest numbers on screen. English and French from the first release.

### Shared by every client

- Compact widget: instance and ping, room, participants with a "speaking"
  indicator and an estimated latency per person, input level meter, mute
  button, gate threshold.
- **Gate calibration**: a 10-second wizard, controller in hand, that sets the
  threshold just above the player's background noise.
- Volume per person, from 0 to 200 %.
- Transparent reconnection, hot-swapping audio devices.
- Warning when a Bluetooth device is active.
- Quality indicator: packet loss, jitter.
- Mute: global keyboard shortcut, tray or menu bar icon, button in the app.

### Presence

- Three short, distinct sounds: someone joined, someone left, someone lost
  their connection (no packet for a few seconds). Adjustable volume, can be
  turned off.
- These sounds are played locally and mixed low. They never delay teammates'
  audio.
- Desktop overlay: a pill such as "Alex joined" or "Alex disconnected" for two
  seconds.
- While in a room: sounds and overlay only, no system notification.
- Not in a room: optional system notification, for example "Sam joined
  #scrims". Local on desktop when the app runs in the tray; push on mobile
  later.
- The control plane broadcasts join, leave and disconnect events over
  Server-Sent Events. Disconnects are detected by the server when UDP packets
  stop arriving.

### Windows and Linux — Tauri v2 + React

- Overlay: a transparent, always-on-top window that lets clicks through. It
  only shows who is speaking, in a corner. The game must run in borderless
  windowed mode. No DirectX injection.
- Tray icon, launch at startup (optional). Mica effect on Windows.
- Linux comes after the other platforms. Overlays are limited under Wayland;
  the Linux app may ship without one.

### iOS and macOS — SwiftUI multiplatform

- `AVAudioEngine` / Core Audio without voice processing, smallest I/O buffer,
  background audio mode.
- iOS: Live Activity and Dynamic Island (mute, leave), interactive widget to
  join or leave a room.
- macOS: menu bar app (`MenuBarExtra`).

### Android — Kotlin + Jetpack Compose

- Foreground service of type `microphone`, to keep voice running with the
  screen off.
- `WifiManager.WifiLock` in `WIFI_MODE_FULL_LOW_LATENCY`. Without it, Wi-Fi
  power saving batches incoming packets and jitter explodes.
- Mute with the wired headset button (media button) and from the
  notification.
- Asks to be excluded from battery optimisation.
- Recommended setup for console players: one wired earbud with an inline
  microphone, worn under the console headset. The gate keeps game sound that
  leaks into the microphone from being sent.

### Web — React + WASM core

- The core runs in an AudioWorklet; transport uses WebTransport datagrams.
- A fallback for people who cannot install an app: a browser always adds
  latency compared to a native client.
- Browser support for WebTransport must be checked on MDN before phase 7.

The visual identity (colours, typography, logo) is defined once the name is
final.

## 11. Measuring latency

The same physical method measures every voice tool: two devices side by
side, a third recorder, and the time between the direct sound and the
received sound.

### Reference method (physical)

1. Device A captures a sharp click with its microphone; device B, connected
   through the voice tool, plays it on a speaker or earphone.
2. A third recorder (a phone is fine) captures both the direct click and the
   replayed one.
3. `drywire-bench measure <recording.wav>` finds each pair of clicks by
   cross-correlation and computes the gaps. At least 20 clicks; we keep the
   median (p50) and p95.

Every measurement is recorded in `docs/measurements.md` with its setup:
devices, connection, frame size, software version. Phase 0 results go to
`docs/baseline.md`.

### Latency lab in the app

- Network latency: UDP round-trip time to the server, continuously.
- Device latency: a loopback test (earphone held against the microphone) that
  measures input plus output, including any wireless headset.
- Estimate shown per person: capture + framing + RTT/2 on each side + jitter
  buffer depth + playback. Always labelled as an estimate.

**Local statistics**: jitter p50/p95, losses, late packets, buffer depth,
audio underruns. Visible in the app, never sent without consent.

**`drywire-bench`** replays simulated network traces (jitter, losses,
bursts) against the jitter buffer. It runs in CI as a regression test.

## 12. Repository, licenses and conventions

All code is MIT-licensed, in one public monorepo from day one. The name and
logo belong to Ligne Native and are not covered by the license.

### Licenses

- Allowed dependency licenses: MIT, BSD, Apache 2.0, ISC, Zlib, MPL 2.0, and
  Unicode-3.0 for the Unicode tables of `unicode-ident` (ADR 0008).
  No GPL, LGPL or AGPL.
- Checked in CI: `cargo-deny` for Rust, `go-licenses` for Go, an equivalent
  check for JavaScript dependencies installed with Bun.
- Root files: `LICENSE`, `README.md`, `CONTRIBUTING.md`, `SECURITY.md`,
  `CODE_OF_CONDUCT.md`, `CLAUDE.md`.

### Layout

```text
drywire/
  Makefile            Entry point for every local and deploy command
  scripts/            What the Makefile runs
  server/             Go: UDP relay + control plane
  core/               Rust workspace
    drywire-proto/    Packet format, end-to-end crypto, binary test vectors
    drywire-core/     Audio I/O, Opus, gate, jitter buffer, stats
    drywire-ffi/      UniFFI bindings (Swift, Kotlin)
    drywire-wasm/     Web build
    drywire-bench/    Network simulation, latency measurement
    drywire-cli/      Command-line client (phase 1)
  apps/
    desktop/          Tauri v2 + React (Windows, Linux)
    apple/            SwiftUI (iOS, macOS)
    android/          Kotlin + Compose
    web/              Web client
  site/               Project site: landing page, invitation page, privacy policy
  docs/               spec.md, protocol.md, baseline.md, measurements.md,
                      privacy.md, adr/ (architecture decision records)
  deploy/             Dockerfile, compose file, systemd unit
```

### Conventions

- **English everywhere in the codebase**: code, comments, commit messages,
  documentation. The user interface is in English and French.
- **Written for newcomers.** Each top-level directory has a `README.md` that
  says what it does and how to run it. Prefer plain, explicit code over clever
  abstractions. Comments explain *why*, not *what*. Every significant decision
  gets a short ADR in `docs/adr/`.
- **Atomic commits.** One logical change per commit; the build and tests pass
  at every commit. The subject line is imperative and short; the body
  explains why.
- `docs/protocol.md` is the source of truth for the wire format. The binary
  vectors in `core/drywire-proto/vectors/` are read by both the Go and Rust
  tests.
- **Semantic versioning.** The protocol version byte changes on every breaking
  change.
- **Version skew is expected.** A client older or newer than the server never
  loses data: control plane changes are additive, the server accepts the
  current and the previous protocol version, and a client facing an
  unsupported version says an update is needed instead of failing silently.
- No secret in the repository. Server configuration through environment
  variables.
- CI on GitHub Actions: Go tests, `cargo test` and `clippy`, license checks,
  builds of every client, `drywire-bench` regression tests.

## 13. Local development and deployment

### Local ports

Development ports are fixed, bound to `127.0.0.1` only, and never fall back
silently to another port. Each can be overridden with an environment
variable.

| Port | Service | Variable |
| --- | --- | --- |
| 10050/tcp | Control plane | `DRYWIRE_HTTP_ADDR` |
| 10050/udp | Relay | `DRYWIRE_UDP_ADDR` |
| 10051 | Vite dev server, desktop UI | `DEV_PORT` |
| 10052 | Project site preview | `SITE_PORT` |
| 10057 | Vite dev server, web client (later) | `WEB_PORT` |

### Commands

The Makefile is thin: each target calls a script in `scripts/`, and `make
help` lists them.

| Command | Effect |
| --- | --- |
| `make help` | List the commands. |
| `make dev` | Run the server locally. |
| `make test` | Run every test suite. |
| `make fmt` | Format all code. |
| `make deploy` | Build the server for Linux, send it to the host, start it. Idempotent. |
| `make status` | Server version, uptime, active rooms. |
| `make logs` | Follow the server logs. |
| `make backup` | Copy an encrypted snapshot of the database to the local machine. |
| `make restore` | Restore the latest backup, or `FILE=…`, after confirmation, then replay deletions. |

### Deployment

- `HOST` is an SSH alias from `~/.ssh/config`. The repository contains no
  host name: each operator sets `HOST` in an untracked `.env` file.
- The developer's machine is the reference. `make deploy` cross-compiles a
  static Linux binary, sends it with `deploy/` to `/srv/drywire` over rsync,
  and starts it with Docker Compose in a `scratch` image. The server never
  pulls from GitHub.
- Drywire owns `/srv/drywire` on the host and touches nothing else, so it
  can share a server with other services.
- The control plane listens on a local port behind the host's reverse proxy,
  which terminates TLS. The UDP port is opened in the host firewall and, if
  any, the provider's firewall.

## 14. Phases and exit criteria

We prove the latency on the command line before drawing any interface. Each
phase ends on a measurable criterion.

0. **Baseline.** Measure the self-hosted voice server between two PCs, the
   same server's Android app with a wired earbud, the console player's current
   chain, and the network paths (`mtr` to the relay from each player's
   connection). **Exit**: results in `docs/baseline.md`.
1. **Low-level prototype.** UDP relay with rooms and the authenticated
   `hello` from a config file (ADR 0004), Rust core, `drywire-cli` on Windows
   and macOS, all three frame sizes, voice gate, `drywire-bench`. Audio is
   encrypted end to end from the start, with keys from a config file
   (ADR 0006). **Exit**: two CLIs talk through the relay with a median below
   the phase 0 baseline on the same setup, for 30 minutes without audible
   artifacts; default frame size chosen and the measurements recorded.
2. **Control plane and security.** Ed25519 identities, spaces and rooms in
   SQLite, invitation links and the static invitation page, which replace
   the phase 1 config files. Data export, identity deletion, retention purge,
   deletion ledger. **Exit**: the team joins by link; a packet capture on the
   server shows only ciphertext;
   latency within 1 ms of phase 1.
3. **Windows app.** Widget, overlay, global mute shortcut, latency lab, gate
   calibration. **Exit**: a full session of matches without falling back to
   another voice tool.
4. **iOS and macOS apps.** Live Activity, widget, menu bar, universal links,
   privacy policy online. **Exit**: a full session from an iPhone and from a
   Mac.
5. **Android app.** **Exit**: the console player plays a full session, with a
   measured latency below the phase 0 Android baseline.
6. **Linux.** The desktop app on PipeWire. **Exit**: a full session on a
   mainstream distribution.
7. **Web client and landing page** on the final domain.
8. **Public opening.** Hosted instance, quotas per space, several relay
   regions, MLS, linking several devices to one identity (QR code), handling
   of minors and impact assessment, features for esport teams (coach channel,
   caster return feed, tournament mode).

## 15. Hosting model and funding

Anyone can self-host the server; later, Ligne Native runs a default instance
so a new group has nothing to install. No monetisation for now, but the data
model keeps per-space quotas possible.

- Self-hosted: one Go binary and one SQLite file.
- Hosted: an official instance, several relay regions as close to players as
  possible.

Price is not a lever: mainstream voice apps are free, and established
self-hosted servers have free tiers. The levers are latency, quality and
privacy. Options to decide later: a free hosted tier with a paid community
plan, paid features for esport teams, or no monetisation at all (sponsorship,
and the project as a showcase for Ligne Native).

## 16. Open decisions

None blocks phase 1.

- [ ] Reference Android phone for the console player, and 3.5 mm versus
      USB-C earbud (blocks phase 5).
- [ ] TLS termination on the relay host: existing reverse proxy or
      certificates handled by the server.
- [ ] Production UDP port, and the firewall rules that open it.
- [ ] Apple Developer and Google Play accounts.
- [ ] Windows code signing: manual installation within the team at first,
      certificate later.
- [ ] Retention of inactive identities: 12 months proposed.
- [ ] Whether receivers need to report downlink loss to senders (answered by
      phase 1 measurements).

## 17. References

- [RFC 6716 — Opus codec](https://www.rfc-editor.org/rfc/rfc6716)
- [RFC 5869 — HKDF](https://www.rfc-editor.org/rfc/rfc5869)
- [RFC 8032 — Ed25519](https://www.rfc-editor.org/rfc/rfc8032)
- [RFC 9420 — Messaging Layer Security (MLS)](https://www.rfc-editor.org/rfc/rfc9420)
- [MDN — WebTransport API](https://developer.mozilla.org/en-US/docs/Web/API/WebTransport_API)
- [Android — Low-latency Wi-Fi lock](https://developer.android.com/reference/android/net/wifi/WifiManager#WIFI_MODE_FULL_LOW_LATENCY)
- [CNIL — Cookies and other trackers](https://www.cnil.fr/en/cookies-and-other-trackers)
