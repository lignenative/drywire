#!/bin/sh
# Follow the server's logs on HOST: the application log (never an address,
# a token or anything a user wrote) and the access log, as JSON lines.
# Ctrl-C stops.
set -eu

cd "$(dirname "$0")/.."
. scripts/host.sh

tty=
[ -t 0 ] && tty=-t
# shellcheck disable=SC2086 # ssh_opts is a list of options
exec ssh $ssh_opts $tty "$HOST" "cd $dir && docker compose logs --follow --tail=100"
