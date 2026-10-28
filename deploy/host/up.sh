#!/bin/sh
# Build the image from the binary beside this file and start the server,
# then wait until its health check passes. Runs on the host, from make
# deploy. The image starts from scratch, so building downloads nothing.
set -eu

cd "$(dirname "$0")/.."

old=$(docker image inspect --format '{{.Id}}' drywire-server:local 2>/dev/null || true)
docker compose up --detach --build --remove-orphans
new=$(docker image inspect --format '{{.Id}}' drywire-server:local)
# The previous image, now unused. Only this one is removed: other
# projects' images on the host are none of Drywire's business.
if [ -n "$old" ] && [ "$old" != "$new" ]; then
	docker image rm "$old" >/dev/null 2>&1 || true
fi

sh host/wait.sh
