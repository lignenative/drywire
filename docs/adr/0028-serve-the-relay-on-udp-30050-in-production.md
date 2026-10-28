# 0028. Serve the relay on UDP 30050 in production

Status: Accepted · 2026-10-28

## Context

The relay and the control plane share port 10050 in development, from the
block of local ports this project uses on a developer's machine. A real
instance needs a UDP port of its own for the relay, open in the host's
and the provider's firewalls. It should not be 10050, so that a
production setting is never mistaken for a development one, and it
should not collide with what else runs on a server or a player's network.

## Decision

The relay listens on **UDP 30050** by default in the deployment
(`DRYWIRE_UDP_ADDR=:30050` in `deploy/instance.env.example`). The port
stays configurable: `DRYWIRE_UDP_ADDR` sets it, `DRYWIRE_PUBLIC_RELAY`
must end with the same port, and `make deploy` checks that they agree,
publishes it and opens it in ufw.

Why 30050:

- It is not assigned to any service in the IANA registry, and no common
  game, voice or streaming service uses it (Steam keeps to 27000–27100,
  TeamSpeak to 9987, Mumble to 64738).
- It is below 32768, where Linux's ephemeral port range starts: an
  outgoing connection of the host can never be holding it when the relay
  starts.
- It keeps "50", Drywire's number in the local port block, so that it
  reads as Drywire's.

The control plane keeps 10050 inside the deployment, where nothing
publishes it: on the host's loopback, or inside the proxy's Docker
network (ADR 0027).

## Consequences

- Operators open 30050/udp, unless they choose another port.
- Development keeps 10050 for both, unchanged.
