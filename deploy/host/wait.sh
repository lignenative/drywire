#!/bin/sh
# Wait up to 30 seconds for the server's health check to pass. On failure,
# print the end of its logs and exit 1.
set -eu

cd "$(dirname "$0")/.."

i=0
while :; do
	health=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' drywire 2>/dev/null || echo missing)
	case $health in
	healthy)
		echo "drywire is running and healthy"
		exit 0
		;;
	starting) ;;
	*)
		[ "$i" -ge 3 ] && break
		;;
	esac
	i=$((i + 1))
	[ "$i" -ge 30 ] && break
	sleep 1
done
echo "drywire is $health; the end of its logs:" >&2
docker compose logs --tail=20 drywire >&2 || true
exit 1
