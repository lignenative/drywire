#!/bin/sh
# Open the relay's UDP port in the host firewall, if the host uses ufw.
# Runs as root through sudo on every deploy; idempotent.
#
# The server runs on the host's own network (compose.yaml), so the
# firewall really decides what is reachable: unlike a published Docker
# port, nothing goes around it. The control plane needs no rule: it only
# listens on 127.0.0.1, and the reverse proxy's ports are the proxy's
# business.
set -eu

cd "$(dirname "$0")/.."

addr=$(sed -n 's/^DRYWIRE_UDP_ADDR=//p' instance.env | tail -n 1)
port=${addr##*:}
port=${port:-10050}

if ! command -v ufw >/dev/null 2>&1; then
	echo "no ufw on this host: open $port/udp in its firewall yourself"
elif ufw status | grep -q '^Status: active'; then
	ufw allow "$port/udp" comment 'drywire relay' >/dev/null
	echo "ufw: $port/udp open for the relay"
else
	echo "ufw is installed but inactive: $port/udp is not filtered by it"
fi
echo "remember the provider's firewall, if any: $port/udp must be open there too"
