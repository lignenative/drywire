# drywire-bench

Replays simulated network traces against the jitter buffer of
`drywire-core`, and reports what the buffer did: the latency it added to
each frame, how often it concealed a missing frame (PLC), how often it
dropped one, and how often a packet came too late to play.

```sh
make bench                    # from the repository root
make bench ARGS="--seed 7"    # another random trace
```

Each scenario simulates two minutes of one person talking, at each frame
size, over one kind of path:

| Scenario | Path |
| --- | --- |
| `clean` | 15 ms one way, no jitter, no loss |
| `wired` | 0.3 ms mean jitter, 0.1 % random loss |
| `wifi` | 2 ms mean jitter, bursts of loss, a 30 ms delay spike every 5 s |
| `bursty-loss` | Losses in bursts (Gilbert-Elliott model), about 0.8 % |
| `bursty-loss+red` | The same, with each packet carrying the previous frame |
| `drift±100ppm` | Wired, with the sender's clock 100 ppm fast or slow, and 10 to 30 s monologues |

The paths are plausible, not measured. Phase 0 records real ones (`mtr` from
each player to the relay); they will replace these.

## How it works

- `src/network.rs`: a seeded random generator, the path (fixed delay,
  exponential jitter, Gilbert-Elliott loss, delay spikes) and the talker.
- `src/simulate.rs`: the sender produces frames on its own clock, the path
  delays or loses each packet, and the receiver calls the real
  `JitterBuffer` once per frame period of its clock, from a random phase.
- `src/scenarios.rs`: the built-in scenarios.

Everything is deterministic: the same seed always gives the same report.

## Regression test

`tests/regression.rs` runs every scenario with seed 1 on each `make test`,
in under a second, and checks bounds: nothing is concealed or dropped on a
clean path, only the lost frames are concealed on a wired one, targets
stay near the floor, redundancy halves concealment, clock drift costs at
most a stretch per talk spurt, and so on. The bounds are what the buffer achieved when they were
written, plus headroom; tighten them when the buffer improves.
