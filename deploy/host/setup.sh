#!/bin/sh
# Open the relay's UDP port in the host firewall, if the host uses ufw.
# Runs as root through sudo on every deploy; idempotent.
#
# On the host's own network (compose.yaml alone), the firewall really
# decides what is reachable, so this rule is what opens the relay. In a
# proxy's Docker network (compose.proxy.yaml), Docker publishes the port
# with its own iptables rule, which goes around ufw: the relay is open
# either way, and this rule only keeps `ufw status` truthful. The control
# plane needs no rule: it is never published, and the reverse proxy's
# ports are the proxy's business.
set -eu

cd "$(dirname "$0")/.."

addr=$(sed -n 's/^DRYWIRE_UDP_ADDR=//p' instance.env | tail -n 1)
port=${addr##*:}
if [ -z "$port" ]; then
	echo "no DRYWIRE_UDP_ADDR in instance.env: no port to open" >&2
	exit 1
fi

if ! command -v ufw >/dev/null 2>&1; then
	echo "no ufw on this host: open $port/udp in its firewall yourself"
elif ufw status | grep -q '^Status: active'; then
	ufw allow "$port/udp" comment 'drywire relay' >/dev/null
	echo "ufw: $port/udp open for the relay"
else
	echo "ufw is installed but inactive: $port/udp is not filtered by it"
fi
echo "remember the provider's firewall, if any: $port/udp must be open there too"
