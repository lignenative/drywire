#!/bin/sh
# Write a consistent snapshot of the database to standard output, while
# the server runs (ADR 0025). Runs on the host, from make backup, which
# encrypts it on the operator's machine: the plain copy never rests on
# disk, here or there. The deletion ledger is left out on purpose
# (ADR 0019).
set -eu

cd "$(dirname "$0")/.."

docker compose exec -T drywire /drywire-server backup
