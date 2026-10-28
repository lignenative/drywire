# 0025. Give the server binary its own maintenance commands

Status: Accepted · 2026-10-28

## Context

The deployment (spec section 13) runs `drywire-server` in a `scratch`
image: one static binary and nothing else, no shell, no `curl`, no
`sqlite3`. Yet an operator needs three things that usually come from those
tools:

- a health check, so Docker and `make status` know the server answers;
- a backup of the SQLite database that is consistent, even while the
  server writes to it;
- a restore that replaces the database and then replays the deletion
  ledger (spec section 9, ADR 0019), so nothing deleted since the backup
  comes back.

Copying the database file while the server runs can catch it halfway
through a write, and its `-wal` file holds changes the main file does not
have yet. Stopping the server for a backup drops every call for a few
seconds.

## Decision

`drywire-server` takes an optional command, reading the same `DRYWIRE_*`
variables as the server:

- `health` asks the control plane on `DRYWIRE_HTTP_ADDR` for `/healthz`
  (through loopback when it listens on every address) and exits 0 if it
  answers. Over TLS it does not check the certificate: it names the public
  host, the image has no CA certificates, and only the request for
  `/healthz` is sent.
- `backup` writes a snapshot of the database to standard output, taken
  with SQLite's `VACUUM INTO` while the server keeps running. The snapshot
  passes through a temporary file beside the database, readable by the
  server's user only, removed at once. The ledger is not in it (ADR 0019).
- `restore` reads a snapshot from standard input into a temporary file,
  checks it (SQLite header, `integrity_check`, schema not newer than this
  server), and only then replaces the database, removing the old `-wal`
  and `-shm` files. It then replays the ledger on it and says how many
  deletions it applied again. It refuses to run while a server answers on
  `DRYWIRE_HTTP_ADDR`.

Encryption is not the binary's job: the deployment scripts pipe the
snapshot through `age` on the operator's machine (ADR 0026).

## Consequences

- Backups need no downtime and no extra image on the host.
- A backup that fails to decrypt, or arrives cut short, leaves the current
  database untouched.
- The server still replays the ledger at every start, so a database put
  back by hand is corrected too; `restore` only does it sooner and reports
  it.
- The restore's running-server check only sees a server on the configured
  address; the deployment scripts stop the service first anyway.
