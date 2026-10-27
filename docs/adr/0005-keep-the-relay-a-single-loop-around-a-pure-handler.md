# 0005. Keep the relay a single loop around a pure handler

Status: Accepted · 2026-10-27

## Context

The relay is on the latency path of every packet. It must be fast, easy to
read, and testable without timing-dependent tests. Common designs use a
goroutine per client, channels, and background timers for expiry.

## Decision

- One goroutine reads the UDP socket. For each datagram it calls
  `Relay.Handle`, which takes the packet and its source address and returns
  the datagrams to send. `Handle` does no I/O.
- The clock is injected. Tests drive `Handle` with a fake clock, so rate
  limits and timeouts are tested exactly, without sleeping.
- There is no background goroutine: idle rate-limit buckets and silent
  sessions are swept at most once per second from `Handle` itself (and from
  `Stats`).
- One mutex guards all state; only the HTTP status handler competes for it.

## Consequences

- The whole data plane reads top to bottom in one function that mirrors
  protocol section 10.1.
- Throughput is bounded by one core. A small team sends a few thousand
  packets per second, far below that. If a large instance ever needs more,
  several sockets with `SO_REUSEPORT` can each run the same loop.
- A session that goes silent while no packet at all reaches the relay is
  unbound on the next packet or status request, not exactly 30 seconds
  later. Nothing observes the difference.
