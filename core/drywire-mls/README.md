# drywire-mls

A space's MLS group (RFC 9420), as ADR 0052 decides it: one group per
space, one leaf per device, and the room keys the audio path will take
from MLS's exporter. Built on OpenMLS (MIT) with its pure-Rust crypto
provider. It does no I/O: the apps carry its messages to and from the
control plane's delivery service ([protocol section
16](../../docs/protocol.md)), which orders them and reads none.

- **`Device`**: this device in the groups: the Ed25519 key it logs in
  with (ADR 0055, `credential.rs`), and the state of all its groups.
  `save` and `load` give that state as bytes for the device's sealed
  store.
- **`Group`**: one space's group. `create`, `join` from a welcome, `add`
  waiting devices, `prune` the devices no longer a member's, `renew` this device's
  leaf; each of these stages a commit, which the app posts and then
  `confirm`s, or `abandon`s when another member's came first. `apply`
  reads another member's commit. `room_key` is the key of a room in the
  current epoch.

Every leaf that comes in is checked: its credential names its own key,
and that key is one of a member's devices, as the control plane lists
them.

```sh
cargo test -p drywire-mls
```

`tests/group.rs` runs a group's life with the delivery service played by
hand, and measures a full space's welcome (about 28 KB for 100 devices).

What does not exist yet (ADR 0052, build order): the audio path taking
its key from here, the apps driving it, the web client's WebAssembly
build, and moving existing spaces.
