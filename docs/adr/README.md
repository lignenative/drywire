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
