# 0019. Replay a deletion ledger at every start

Status: Accepted · 2026-10-27

## Context

Phase 2 stores identities, spaces and memberships in SQLite. The spec
(section 9) requires that a deletion survives a restore: if an operator
restores a backup taken before someone deleted their identity, that
identity must not come back. Backups are kept up to six months.

The ledger that makes this possible is itself a record of deletions, so it
must say as little as possible about the people it lists.

## Decision

- Every deletion of personal data is first appended to a **deletion
  ledger**, a JSON-lines file beside the database (`DRYWIRE_LEDGER`), synced
  to disk before the database changes. A crash between the two leaves an
  entry for a deletion the database has not done yet, which the next replay
  finishes: the safe direction.
- Three kinds of entries: an identity, a space, and one identity's
  membership of one space. An identity is named by the **SHA-256 of its
  public key**, never the key: the ledger can recognise a returning row, not
  say who it was. Public keys are 256-bit random values, so the hash cannot
  be reversed by trying candidates.
- The server **replays the whole ledger at every start**, not only after a
  restore. Replay is idempotent and cheap for a team-sized instance, and an
  operator cannot forget a step that does not exist.
- An entry only deletes rows **created before it**: someone who deleted
  their identity and later joined again with the same key keeps the new
  identity.
- The ledger is never part of a backup, and entries older than seven months
  (the six-month backup limit plus a margin) are pruned at start.
- The schema is a list of numbered migrations in Go, tracked by SQLite's
  `user_version`. A server refuses a database written by a newer one. One
  database connection serialises every write; a control plane for a few
  teams needs no more.

## Consequences

- `make restore` (deployment, later) needs no special step: restarting the
  server is the replay.
- Bans are not replayed: they are not a deletion of data about the banned
  person but a decision by the space, and a restore that brings back an old
  ban list is the operator's call.
- Losing the ledger file loses the guarantee for backups older than the
  loss. The deployment keeps it on the same volume as the database, outside
  the backup set.

## Amendment · 2026-10-28

After the security review of the phase 2 slice:

- A fourth kind of entry records a **revoked invitation**, so a restored
  backup cannot make a leaked link work again. A member who is removed
  from a space, or is no longer an admin, loses the invitations they
  created there, so they cannot come back with a code of their own.
- An entry only deletes rows created **strictly before** its second: a
  member who leaves and joins again within the same second keeps the new
  membership. The opposite case (a row created and deleted in the same
  second, inside a backup taken in that second) is left alone.
- A last line cut short by a crash during an append is skipped rather
  than stopping the server; its deletion had not happened yet.
- Bans are still not replayed: restoring an older backup drops the bans
  made since, and a banned key holding a still-valid code could redeem it.
  Revoke invitations after a restore if that matters.
