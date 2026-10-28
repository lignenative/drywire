# 0027. Let a proxy on a private network reach the control plane

Status: Accepted · 2026-10-28

## Context

Bearer tokens cross the control plane's connections, so in plain HTTP it
only listened on a loopback address: TLS came from a reverse proxy on the
same host, and `DRYWIRE_TRUST_PROXY=1` believed `X-Forwarded-For` from
loopback peers only.

Many hosts run their reverse proxy in a container, on a Docker network
shared by the applications it serves (Caddy, Traefik, nginx). Such a proxy
cannot reach the host's `127.0.0.1`, and its connections come from a
private address of that network, never from loopback. With the loopback
rule alone, Drywire could not sit behind it without serving TLS itself,
which would mean a second certificate for the same name.

## Decision

- `DRYWIRE_TRUST_PROXY` takes either `1` (a proxy on this host's loopback,
  as before) or a comma-separated list of **private** networks or
  addresses, such as `172.18.0.0/16`: the Docker network of the proxy.
  Only loopback and the private ranges of RFC 1918 and RFC 4193 are
  accepted; a public network is refused at start, since anyone there could
  forge the header. Loopback stays trusted, for the health check.
- In plain HTTP, the control plane may listen on a non-loopback address
  only when such a network is named. It then **accepts connections from
  the named networks and loopback alone**, and closes any other at once.
  Tokens cross that private network unencrypted, between the proxy and
  the server, and reach nobody else.
- The deployment keeps the host network by default (ADR 0026). With
  `DRYWIRE_PROXY_NETWORK` set in the operator's `.env`, `make deploy`
  looks up that network's subnet on the host and adds
  `deploy/compose.proxy.yaml`: the server joins the network, its HTTP
  port is not published, the relay's UDP port is, and
  `DRYWIRE_TRUST_PROXY` is set to the subnet.

## Consequences

- An operator with a containerized proxy routes to `drywire:10050` on the
  shared network, with no certificate of Drywire's own.
- The relay's packets then go through Docker's port forwarding, a kernel
  NAT rule: IPv4 clients keep their address, and the cost is far below a
  millisecond, but it is one more step than the host network. A published
  port also goes around ufw.
- Every other container on the proxy's network can connect to the control
  plane in plain HTTP. It can do no more than any client, but it could
  read the tokens of others only by intercepting that network, which
  needs control of the host. Keep that network to applications you trust.
