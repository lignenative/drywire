#!/bin/sh
# Build the images from the binaries and files beside this file and start
# the server and the web client's files, then wait until their health
# checks pass. Runs on the host, from make
# deploy. The image starts from scratch, so building downloads nothing.
set -eu

cd "$(dirname "$0")/.."

images="drywire-server:local drywire-static:local"
old=$(for i in $images; do docker image inspect --format '{{.Id}}' "$i" 2>/dev/null || true; done)
docker compose up --detach --build --remove-orphans
new=$(for i in $images; do docker image inspect --format '{{.Id}}' "$i"; done)
# The previous images, now unused. Only Drywire's are removed: other
# projects' images on the host are none of Drywire's business.
for id in $old; do
	case $new in
	*"$id"*) ;;
	*) docker image rm "$id" >/dev/null 2>&1 || true ;;
	esac
done

sh host/wait.sh
