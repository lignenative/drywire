# web

Drywire in a browser, for people who will not install an app first. It
will always be slower than a native client; it is the easy door, not
the way to play (spec section 10).

For now this is a **developer's page**, not the interface: it proves the
whole path. It joins a room with an invitation link, binds its session
to the relay over WebTransport, sends one second of a 440 Hz tone as
end-to-end encrypted Opus frames, and decodes what the other members send
back. The real interface will reuse the desktop app's React components,
drawn on the design canvas first (CLAUDE.md, "Design first").

## Try it

```sh
rustup target add wasm32-unknown-unknown   # once
make web
```

`make web` builds the server, the command-line client and the
WebAssembly module, starts a server with its WebTransport side
(`127.0.0.1:10058/udp`), puts an echo bot in a room, and serves this page
with Vite on `127.0.0.1:10057`. Open the address it prints (the
invitation is in its fragment, which the browser never sends anywhere)
and press the button. The page prints what happened:

- `pingMs`: round trips to the relay over WebTransport.
- `sent`, `received`: our frames, and the echo bot's frames that
  decrypted.
- `toneHz`: the frequency of what came back, 440 if the tone survived.
- `firstEchoMs`: from our first frame to the first echoed one. It holds
  the echo bot's jitter buffer and its own frames: it is **not** a
  mouth-to-ear latency, which is measured with the reference method
  (spec section 11).

It needs a browser with WebTransport and WebCodecs Opus: Chrome or Edge
110+, Firefox 130+ on the desktop, Safari 26.4+.

## How it is built

| File | What it does |
| --- | --- |
| `src/protocol.ts` | `drywire-wasm` ([`core/drywire-wasm/`](../../core/drywire-wasm/)): the protocol and the encryption, in WebAssembly (ADR 0040) |
| `src/identity.ts` | The device's Ed25519 key, non-extractable, in WebCrypto and IndexedDB |
| `src/control.ts` | Login, invitation, join (protocol section 13) |
| `src/relay.ts` | The WebTransport session, pinned by the certificate hashes the join answer gives (ADR 0038, 0039) |
| `src/codec.ts` | Opus through WebCodecs, for now (ADR 0040) |
| `src/main.ts` | The page's run, from link to echo |

In development the page reaches the control plane through Vite's proxy
(`/v1`): the control plane does not answer cross-origin requests yet.

## Test

```sh
bun test        # the WebAssembly module against core/drywire-proto/vectors/
bun run typecheck
```

`make test` builds the module into `public/` and runs both.
