# 0049. Serve the web client from each instance, beside its API

Status: Accepted · 2026-11-04

## Context

The web client (`apps/web/`, ADR 0048) holds a call through a real
instance, but only from `make web`, where Vite serves the page and
forwards `/v1` to a local server. To try it from a phone or another
computer, an instance must serve it. Two questions were left open in the
spec (section 16):

- **Where the page comes from.** Either each instance serves it, at its
  own address, or the project's domain serves one copy for every
  instance. From the project's domain, every request to the control
  plane is cross-origin: the control plane would have to answer CORS
  preflights, the login's audience (ADR 0020) would differ from the
  page's origin, and one page would have to talk to instances of any
  version.
- **Which UDP port the relay's WebTransport side uses in production**
  (ADR 0038: 10058 in development).

Two rules frame the answer. The Go server is an API only: it never
serves the front. And a deployment downloads nothing on the host: `make
deploy` builds everything on the operator's machine and the images
start from `scratch` (ADR 0026).

## Decision

- **Each instance serves its own web client, at its public URL.** The
  reverse proxy sends `/v1/…` to the control plane and every other path
  to the web client's files. Page and API share one origin: no CORS, the
  login is signed for the page's own origin, and the page always comes
  from the same commit as the server it talks to. An invitation opens in
  a browser as `https://<instance>/#<payload>`, the same fragment as the
  project's link.
- **The files are a service of their own**, `drywire-static`
  (`server/cmd/drywire-static`): some 150 lines of Go's standard
  library that serve a directory, list none, accept only `GET` and
  `HEAD`, keep no log of requests, and tell browsers to keep Vite's
  hashed `assets/` for a year and to check everything else (the page,
  the WebAssembly module) on every load. It runs in its own container,
  `drywire-web`, from its own `scratch` image with the client's build,
  on `127.0.0.1:10052` of the host, or `drywire-web:10052` in a proxy's
  Docker network (ADR 0027). 10052 is the place the local port block
  gives a built front served alone. A web server image (Caddy, nginx)
  would have to be downloaded on the host, and the proxy itself cannot
  read files that live in `/srv/drywire` when it runs in a container.
- **The WebTransport side listens on UDP 30058 in production**, as 30050
  is the relay's (ADR 0028): the development port plus 20000, below
  Linux's ephemeral range, unassigned. `make deploy` checks that
  `DRYWIRE_PUBLIC_WEB_RELAY` ends with `DRYWIRE_WEB_ADDR`'s port, opens it
  in ufw, and publishes it when the server sits in a proxy's network.
- **A deployed instance always serves browsers**: `make deploy` refuses
  an `instance.env` without `DRYWIRE_WEB_ADDR`. The server itself keeps
  browsers opt-in (ADR 0038), for whoever runs it some other way.
- `make deploy` builds the WebAssembly module and the web client with
  Bun on the operator's machine, as `make test` does, and sends the
  build with the binaries.

## Consequences

- Operators open one more UDP port, 30058, beside 30050.
- The operator's machine needs Bun and Rust's `wasm32-unknown-unknown`
  target to deploy, not only Go.
- `/healthz` is no longer reachable through the proxy; it never needed
  to be (the health checks run inside the containers).
- A page loaded before a deploy keeps running the old code until it is
  reloaded; the next load checks the page and gets the new one. The
  server accepts the previous protocol version (CLAUDE.md), so an old
  page still joins.
- The invitation page on the project's domain (`site/i/`) cannot know
  which instances serve a web client; offering "open in your browser"
  there, with the instance taken from the fragment, is left for later.
- The systemd unit (`deploy/drywire.service`) still runs the server
  alone; a host without Docker serves `apps/web`'s build with its own
  web server.
