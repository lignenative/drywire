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

# The SSH session may be gone by the time the server starts again (Ctrl-C
# on the operator's side, a dropped connection): this script must not die
# of it, and the commands that start the server write to a log beside it,
# shown afterwards to whoever is still there.
trap '' HUP PIPE
start_again() {
	status=$?
	{ docker compose up --detach drywire && sh host/wait.sh; } >restore.log 2>&1 || status=1
	cat restore.log >&2 || true
	exit "$status"
}
docker compose stop drywire >&2
trap start_again EXIT
# A one-off container with the server's volume and settings. Its own
# check for a running server only sees its own network (on a proxy's
# Docker network, not the server's): the stop above is what counts.
docker compose run --rm -T --no-deps drywire restore
