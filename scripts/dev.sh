#!/bin/sh
# Run the server (relay and control plane) on this machine.
#
# It listens on 127.0.0.1:10050 for both TCP (HTTP) and UDP (audio), and on
# 127.0.0.1:10058 UDP for browsers (WebTransport), unless DRYWIRE_HTTP_ADDR,
# DRYWIRE_UDP_ADDR or DRYWIRE_WEB_ADDR say otherwise. If a port is taken the
# server stops with an error instead of picking another one.
set -eu

cd "$(dirname "$0")/../server"

# The database lives in .dev/ at the repository root, which git ignores.
mkdir -p ../.dev
: "${DRYWIRE_DB:=../.dev/drywire.db}"
# Anyone may create a space on a development server.
: "${DRYWIRE_SPACE_CREATORS:=*}"
# Browsers reach the relay over WebTransport on a UDP port of its own.
: "${DRYWIRE_WEB_ADDR:=127.0.0.1:10058}"
# Sign-in codes (ADR 0055) are written to files there, never sent.
: "${DRYWIRE_MAIL_DIR:=../.dev/mail}"
export DRYWIRE_DB DRYWIRE_SPACE_CREATORS DRYWIRE_WEB_ADDR DRYWIRE_MAIL_DIR

exec go run ./cmd/drywire-server
