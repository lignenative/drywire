# 0038. Reach the relay from browsers over WebTransport, on a port of its own

Status: Accepted · 2026-11-04

## Context

The web client moves from phase 7 to right after the Windows app: it is
the door for people who will not install anything before trying Drywire.
It stays a fallback, never as fast as a native client (spec section 5).

A browser cannot send UDP. What it can send without ordering or
retransmission, which voice needs, is:

- **WebTransport datagrams** (RFC 9221 over HTTP/3). In every engine
  since Safari 26.4 shipped it in March 2026: Chrome and Edge 97,
  Firefox 114, Safari 26.4 on macOS and iOS
  ([MDN compatibility data](https://github.com/mdn/browser-compat-data),
  [WebKit, Safari 26.4](https://webkit.org/blog/17862/webkit-features-for-safari-26-4/)).
  A client-server protocol: the browser connects to one server.
- **WebRTC data channels** (SCTP over DTLS over ICE), unordered and
  without retransmission. Everywhere for years, but the server side is a
  whole WebRTC stack (ICE, DTLS, SCTP), and its Go implementation is a
  large dependency tree.

The spec already planned WebTransport and accepted `quic-go` for it.
Serving WebTransport from Go takes two modules, both MIT:

- [`quic-go`](https://github.com/quic-go/quic-go) for QUIC and HTTP/3,
  with RFC 9221 datagrams;
- [`webtransport-go`](https://github.com/quic-go/webtransport-go), from
  the same authors, for the WebTransport session on top (draft-ietf-
  webtrans-http3-16, and the older settings that Chrome, Firefox and
  Safari still send). Used by libp2p, Caddy and MediaMTX.

What they bring in is all BSD-3 or MIT: `quic-go/qpack`,
`dunglas/httpsfv`, `golang.org/x/crypto`, `x/net`, `x/text`, `x/sys`
(checked with `go-licenses`). Everything is pure Go: the server stays a
static binary.

QUIC could share the relay's UDP socket: `quic-go` hands non-QUIC
packets back (`Transport.ReadNonQUICPacket`), and our version byte, `1`,
never looks like QUIC. But every native packet would then go through
`quic-go`'s read loop, a copy and a channel, into a queue of 32 that
drops silently when full.

## Decision

- **Browsers send the same packets** as native clients (protocol
  sections 3 to 10), one per WebTransport datagram. Nothing on the wire
  changes, nothing is translated, and the relay stays blind.
- **A session is bound to a WebTransport session** by the same `hello`
  a native client sends over UDP: the relay binds it to that connection
  instead of an address (`relay.Link`), and copies packets between
  browsers and native clients alike. A WebTransport session stays the
  same client when QUIC moves it to another address.
- **WebTransport gets a UDP port of its own** (`DRYWIRE_WEB_ADDR`,
  10058 in development). The native clients' socket and loop do not
  change: their latency cannot regress because of browsers.
- **Opt-in.** A server without `DRYWIRE_WEB_ADDR` serves no browser,
  and its join answer has no `web_relay`. `make dev` turns it on.
- `quic-go` is pinned to **v0.64.1**: `webtransport-go` v0.13.0 does not
  build against v0.65.0, which changed `HTTPStream`'s signature. Move
  both together when `webtransport-go` catches up.
- **Any origin** may open a session: nothing rides on the connection
  (no cookie), and nothing happens before a `hello` proves the session
  secret.

## Consequences

- Two more Go modules (and their dependencies) in the server, accepted
  by the spec; the binary grows to about 20 MB.
- An operator who wants browsers opens one more UDP port. Its production
  number, and its place in the deployment, are decided when the hosted
  instance serves browsers (spec section 16).
- WebTransport needs TLS. How the relay gets a certificate browsers
  accept is ADR 0039.
- Browsers write datagrams differently: Chrome has only
  `datagrams.writable`, Safari only `datagrams.createWritable()`, Firefox
  both. The web client detects which.
