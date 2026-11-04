#!/bin/sh
# Make a creation code on HOST: it lets one person create one space there,
# within 7 days, without their key in DRYWIRE_SPACE_CREATORS (ADR 0054).
# The server keeps only its hash; hand the code over privately.
set -eu

cd "$(dirname "$0")/.."
. scripts/host.sh

on_host "docker compose exec -T drywire /drywire-server creator-code"
