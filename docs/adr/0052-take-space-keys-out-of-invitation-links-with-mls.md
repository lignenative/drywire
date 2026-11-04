# 0052. Take space keys out of invitation links with MLS

Status: Accepted · 2026-11-04 · steps 1 to 4 built; spaces with a space key stay as they are

## Context

A space's key is 256 random bits that travel in the fragment of every
invitation link, and every room and sender key derives from it (spec
section 8). The relay can neither read nor forge the audio, which the
lock shows (ADR 0041). Four limits remain, and the spec names them:

- **A link holds the key for good.** A forwarded link, a screenshot or a
  chat history gives the key to whoever sees it, even after the code has
  expired or been used up: the code only gates membership on the server.
- **Removing a member does not take the key back.** The space must get a
  new key and everyone must be invited again, by hand.
- **No forward secrecy.** Whoever recorded the encrypted audio, and
  learns the space key one day, decrypts everything recorded before.
- **The server says which key is whose.** A dishonest instance could list
  a key of its own as a member. Nothing yet lets two people compare their
  keys out of band.

Messaging Layer Security (RFC 9420) is the standard answer for groups:
each epoch's secret goes only to the group's current devices, encrypted
to each; adding or removing a device starts a new epoch, so a removed
device learns nothing after, and a leaked epoch secret opens nothing
before. OpenMLS 0.9.2 implements it in Rust under the MIT license
(checked with `cargo info`). It compiles to WebAssembly for the web
client (checked 2026-11-04: a scratch crate that creates a group and
draws an exporter secret, compiled, not yet run in a browser) only with
its `js` feature: `wasm-bindgen`, `js-sys` and `web-time`, and
`getrandom` 0.2 and 0.4 set to take the browser's randomness. The
module then weighs 2.1 MB, against 187 KB for `drywire-wasm` today,
which loads with no glue code.

## Decision

The direction, to build after the accounts of ADR 0051:

- **One MLS group per space**, whose members are the accounts' devices,
  each a leaf of its own.
  The control plane is MLS's delivery service: it stores and orders the
  handshake messages (proposals, commits, welcomes) of each space, and
  accepts one commit per epoch. It reads none of them: the Go server
  needs no MLS code, only storage and ordering.
- **Each device signs as itself in the group.** A spike (2026-11-04,
  OpenMLS 0.9.2) confirmed what RFC 9420 requires: two leaves cannot
  share a signature key ("Duplicate signature key in proposals and
  group"). Since ADR 0055 each device has an Ed25519 key of its own, the
  one it logs in with, and that key signs for it in every group; its
  basic credential's identity is that same public key. A member accepts
  a leaf only if its identity is its signature key and the control plane
  lists that key among a member's devices. (Before ADR 0055 the key was
  the account's, shared by its devices, and each device was to have an
  MLS key of its own certified by it; that went with the shared key.)
- **The room key comes from the group**, through MLS's exporter
  (`room_key = MLS-Exporter("drywire/room/v1", room_id, 32)` in the
  current epoch). Everything after it does not change: per-sender keys,
  salts, signed announcements and AES-GCM per frame (protocol section 8).
  **The audio path gains nothing to do**: key changes happen beside it,
  as a new epoch, which senders already handle (ADR 0002).
- **Invitation links carry the code only.** Redeeming it makes the
  newcomer a member on the server and posts their key package; the app of
  any member online adds them to the group at once, without asking
  anyone. With nobody online, the newcomer waits, and the app says so.
- **Removing or banning a member removes their devices from the group**,
  which starts a new epoch at once.
- **Every device renews its own key material** at least once a day while
  it runs, for post-compromise security.
- **Safety codes** compare the account keys (ADR 0053, built): one check
  per pair of people covers every device, since each leaf is certified by
  its account key.
- **Existing spaces** keep their key from the link until their owner
  moves them to a group; links made after that carry no key.

## Build order

The same spike ran the group's life natively: three leaves (two devices
of one account, one of another) share one room key; removing one starts
epoch 2 with a new key, which the removed leaf cannot derive. What
remains, in order, each a slice with its tests:

1. **Delivery service** on the control plane (built, protocol section
   16): key packages per pending device, commits accepted only for the
   current epoch (`409` otherwise), one welcome per commit for the
   devices it adds, an `mls` event on the space's stream, all stored as
   opaque bytes and deleted with their space, member or identity, or
   when stale.
2. **`drywire-mls`** in the core (built): device credentials as above,
   the group kept in each device's sealed store (OpenMLS's memory
   storage, saved as a whole), adding pending devices, removing members'
   devices, renewing its own leaf.
3. **The audio path's key**: the room key from the exporter per epoch,
   the previous one kept so packets in flight still open, and
   `drywire-core` taking a room key instead of the space key. Built in
   the core: a call takes a room key, and the next one with
   `CallMembers::set_room_key` (protocol section 8.3).
4. **The apps** (built, with ADR 0055's accounts): the CLI, the desktop
   app and the web client run `SpaceGroup` (drywire-control) or its
   port (apps/web/src/group.ts) before a call and on every `mls` event.
   The browser loads OpenMLS as a second module, 3.9 MB (870 KB
   compressed), with wasm-bindgen's glue; it rejoins the room on a new
   epoch instead of switching keys in flight.
5. **Moving existing spaces**, then links without the key.

## Consequences

- A link that leaks after its code is used or expired gives nothing, and
  removing someone takes effect at once, without new invitations.
- Joining needs a member's app online once, to hand over the epoch
  secret. A small team is rarely all offline; a space whose members never
  meet would see newcomers wait.
- **Still out of reach**: a member can forge audio in another member's
  name, since every member holds the room key. Signing each 5 ms frame
  would add 64 bytes per packet, about 100 kb/s per speaker, which a
  group of friends does not need.
- The web client needs MLS in WebAssembly: it builds, at the cost of
  `wasm-bindgen`'s glue code and a module about ten times larger, which
  the page can load after the interface, before the first join.
