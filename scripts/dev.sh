#!/bin/sh
# Run the relay on this machine with the development rooms.
#
# It listens on 127.0.0.1:10050 for both TCP (HTTP) and UDP (audio) unless
# DRYWIRE_HTTP_ADDR or DRYWIRE_UDP_ADDR say otherwise. If a port is taken the
# server stops with an error instead of picking another one.
set -eu

cd "$(dirname "$0")/../server"

: "${DRYWIRE_ROOMS:=rooms.dev.json}"
# The database lives in .dev/ at the repository root, which git ignores.
mkdir -p ../.dev
: "${DRYWIRE_DB:=../.dev/drywire.db}"
# Anyone may create a space on a development server.
: "${DRYWIRE_SPACE_CREATORS:=*}"
export DRYWIRE_ROOMS DRYWIRE_DB DRYWIRE_SPACE_CREATORS

exec go run ./cmd/drywire-server
