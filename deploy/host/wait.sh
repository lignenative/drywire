#!/bin/sh
# Wait up to 30 seconds for the health checks of the server and of the web
# client's files to pass. On failure, print the end of the failing
# container's logs and exit 1.
set -eu

cd "$(dirname "$0")/.."

# wait_healthy CONTAINER SERVICE
wait_healthy() {
	i=0
	while :; do
		health=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "$1" 2>/dev/null || echo missing)
		case $health in
		healthy)
			echo "$1 is running and healthy"
			return 0
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
	echo "$1 is $health; the end of its logs:" >&2
	docker compose logs --tail=20 "$2" >&2 || true
	exit 1
}

wait_healthy drywire drywire
wait_healthy drywire-web web
