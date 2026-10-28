# 0026. Run the server in a scratch image on the host network

Status: Accepted · 2026-10-28

## Context

The team's first real test needs a deployed instance behind TLS. The spec
(section 13) sets the model, the operator's homelab one: a thin Makefile,
an SSH alias `HOST`, rsync to `/srv/drywire`, Docker Compose, and a host
that never pulls from GitHub. Three choices were left open: how the image
is built, how the container reaches the network, and how backups are
encrypted.

## Decision

- **The binary is built on the operator's machine** (`CGO_ENABLED=0`, for
  the host's processor) and sent with `deploy/` over rsync. The image is
  `FROM scratch` plus that binary, run as an unprivileged user, its state
  in one volume. Building it on the host downloads nothing: no base image,
  no Go toolchain, no module.
- **The container uses the host's network** (`network_mode: host`), not
  Docker's bridge:
  - audio packets reach the relay with no NAT rule or userland proxy in the
    way, which is the lowest latency the host can give (Principles:
    latency first);
  - the relay sees each client's real address and port;
  - the control plane listens on the host's `127.0.0.1`, where only a
    reverse proxy on the same host reaches it, and `DRYWIRE_TRUST_PROXY`
    can tell that proxy (a loopback peer) apart from everyone else. On a
    bridge, the proxy's connections would come from a Docker gateway
    address instead.
  - Docker's published ports go around ufw; a host-network server does
    not, so the host firewall says the truth about what is open.
- **Backups are encrypted with age on the operator's machine.** The server
  streams a snapshot over SSH (ADR 0025) straight into `age`, so the plain
  copy never rests on a disk. The operator provides the recipients' public
  keys and keeps the identity off the server. age is a separate tool the
  operator installs, not a dependency of Drywire.
- A systemd unit is the alternative for a host without Docker; the `make`
  targets drive the Docker deployment only.

## Consequences

- The host needs Docker Engine 25 or later (health check `start_interval`)
  with Compose, and nothing else of Drywire's; rsync is installed if
  missing.
- Docker Desktop has no host network by default, so trying the deployment
  on a Mac uses `deploy/compose.local.yaml`, where a Caddy container plays
  the host.
- The relay port is whatever `DRYWIRE_UDP_ADDR` binds; the operator opens
  it in the host's and the provider's firewalls (`make deploy` does the
  first when the host uses ufw).
