#!/bin/sh
# Print the state of the server's container. Runs on the host, from make
# status; the version, uptime and rooms come from the public URL.
set -eu

cd "$(dirname "$0")/.."

if ! docker inspect drywire >/dev/null 2>&1; then
	echo "no drywire container: run make deploy" >&2
	exit 1
fi
docker inspect --format 'container  {{.State.Status}}{{if .State.Health}}, {{.State.Health.Status}}{{end}}, started {{.State.StartedAt}}, restarted {{.RestartCount}} times' drywire
docker stats --no-stream --format 'resources  CPU {{.CPUPerc}}, memory {{.MemUsage}}' drywire
