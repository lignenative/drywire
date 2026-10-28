#!/bin/sh
# Replace the database with the snapshot read from standard input, then
# replay the deletion ledger on it (ADR 0019, ADR 0025). Runs on the host,
# from make restore, after the operator confirmed.
#
# The server is stopped meanwhile, and started again whatever happens: on
# the restored database if the restore worked, on the untouched current
# one if it did not (drywire-server restore checks the snapshot before
# replacing anything). The ledger stays where it is, in the volume.
set -eu

cd "$(dirname "$0")/.."

start_again() {
	status=$?
	docker compose up --detach drywire >&2
	sh host/wait.sh >&2 || status=1
	exit "$status"
}
docker compose stop drywire >&2
trap start_again EXIT
# A one-off container with the server's volume and settings.
docker compose run --rm -T --no-deps drywire restore
