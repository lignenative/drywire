# Architecture decision records

Each file records one significant decision: its context, what we chose, and
what follows from it. Records are numbered in order and never rewritten once
accepted; a later record supersedes an earlier one instead.

A record is short and has four parts:

```markdown
# NNNN. Title in the imperative

Status: Accepted · YYYY-MM-DD

## Context
## Decision
## Consequences
```

| Number | Decision |
| --- | --- |
| [0001](0001-size-replies-to-their-requests.md) | Size every reply to its request |
| [0002](0002-restart-the-audio-sequence-per-key-epoch.md) | Restart the audio sequence at each key epoch |
| [0003](0003-generate-the-test-vectors-in-go.md) | Generate the shared test vectors in Go |
| [0004](0004-configure-phase-1-rooms-with-a-json-file.md) | Configure phase 1 rooms with a JSON file |
| [0005](0005-keep-the-relay-a-single-loop-around-a-pure-handler.md) | Keep the relay a single loop around a pure handler |
| [0006](0006-encrypt-audio-from-phase-1.md) | Encrypt audio from phase 1 |
