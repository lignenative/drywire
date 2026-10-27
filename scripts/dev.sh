#!/bin/sh
# Run the relay on this machine with the development rooms.
#
# It listens on 127.0.0.1:10050 for both TCP (HTTP) and UDP (audio) unless
# DRYWIRE_HTTP_ADDR or DRYWIRE_UDP_ADDR say otherwise. If a port is taken the
# server stops with an error instead of picking another one.
set -eu

cd "$(dirname "$0")/../server"

: "${DRYWIRE_ROOMS:=rooms.dev.json}"
export DRYWIRE_ROOMS

exec go run ./cmd/drywire-server
