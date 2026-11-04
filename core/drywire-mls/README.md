# drywire-mls

A space's MLS group (RFC 9420), as ADR 0052 decides it: one group per
space, one leaf per device, and the room keys the audio path will take
from MLS's exporter. Built on OpenMLS (MIT) with its pure-Rust crypto
provider. It does no I/O: the apps carry its messages to and from the
control plane's delivery service ([protocol section
16](../../docs/protocol.md)), which orders them and reads none.

- **`Device`**: this device's own Ed25519 key, certified by the account
  key (`credential.rs` says how), and the state of all its groups.
  `save` and `load` give that state as bytes for the device's sealed
  store; it never goes into the account's vault.
- **`Group`**: one space's group. `create`, `join` from a welcome, `add`
  waiting devices, `remove` an account's devices, `renew` this device's
  leaf; each of these stages a commit, which the app posts and then
  `confirm`s, or `abandon`s when another member's came first. `apply`
  reads another member's commit. `room_key` is the key of a room in the
  current epoch.

Every leaf that comes in is checked: certified by the account it names,
and that account a member of the space, as the control plane lists it.

```sh
cargo test -p drywire-mls
```

`tests/group.rs` runs a group's life with the delivery service played by
hand, and measures a full space's welcome (about 28 KB for 100 devices).

What does not exist yet (ADR 0052, build order): the audio path taking
its key from here, the apps driving it, the web client's WebAssembly
build, and moving existing spaces.
