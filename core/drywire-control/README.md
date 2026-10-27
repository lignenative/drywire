# drywire-control

The client side of Drywire's control plane: log in with the device's
Ed25519 identity, create spaces and invitation links, redeem them, join a
room, and follow who comes and goes. Blocking HTTPS through `ureq` and
`rustls`, trusting the operating system's certificates (ADR 0022).

It does no audio. Joining a room gives a `JoinAnswer`: the UDP session,
its secret, the relay's address and everyone in the room, which is what
`drywire-core` needs to start a call.

| File | What it does |
| --- | --- |
| `src/lib.rs` | `Client`: login (protocol section 13.2) and one method per request |
| `src/types.rs` | The JSON the server sends and receives, as Rust types |
| `src/events.rs` | Reads a space's Server-Sent Events stream |

## Test it

```sh
cargo test -p drywire-control
```

`tests/control_plane.rs` runs the real server (built from `server/` with
Go, like the other integration tests) and walks through a space, an
invitation link, two members in a room, presence events, and deleting an
identity.
