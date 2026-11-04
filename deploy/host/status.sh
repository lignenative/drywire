#!/bin/sh
# Print the state of the containers: the server's and the web client's.
# Runs on the host, from make status; the version, uptime and rooms come
# from the public URL.
set -eu

cd "$(dirname "$0")/.."

if ! docker inspect drywire >/dev/null 2>&1; then
	echo "no drywire container: run make deploy" >&2
	exit 1
fi
# show CONTAINER LABEL: its state, then what it uses.
show() {
	docker inspect --format "$2"'{{.State.Status}}{{if .State.Health}}, {{.State.Health.Status}}{{end}}, started {{.State.StartedAt}}, restarted {{.RestartCount}} times' "$1"
	# docker stats trims the spaces a format starts with: printf indents.
	printf '             '
	docker stats --no-stream --format 'CPU {{.CPUPerc}}, memory {{.MemUsage}}' "$1"
}

show drywire 'server       '
if docker inspect drywire-web >/dev/null 2>&1; then
	show drywire-web 'web client   '
else
	echo "web client   no drywire-web container: run make deploy"
fi
