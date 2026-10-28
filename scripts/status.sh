#!/bin/sh
# Show the server on HOST: its container, then its version, uptime and
# active rooms as clients see them, through the public URL.
set -eu

cd "$(dirname "$0")/.."
. scripts/host.sh

echo "==> $HOST"
on_host "sh host/status.sh"

public_url=$(instance_setting DRYWIRE_PUBLIC_URL)
[ -n "$public_url" ] || fail "no DRYWIRE_PUBLIC_URL in deploy/instance.env"
echo "==> $public_url"
printf 'version    '
curl -fsS --max-time 10 "$public_url/v1/version"
printf 'status     '
curl -fsS --max-time 10 "$public_url/v1/status"
