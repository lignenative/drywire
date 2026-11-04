#!/bin/sh
# Open the relay's UDP ports in the host firewall, if the host uses ufw:
# the native clients' one and the browsers' one. Runs as root through
# sudo on every deploy; idempotent.
#
# On the host's own network (compose.yaml alone), the firewall really
# decides what is reachable, so this rule is what opens the relay. In a
# proxy's Docker network (compose.proxy.yaml), Docker publishes the ports
# with its own iptables rules, which go around ufw: the relay is open
# either way, and these rules only keep `ufw status` truthful. The control
# plane and the web client's files need no rule: they are never published,
# and the reverse proxy's ports are the proxy's business.
set -eu

cd "$(dirname "$0")/.."

# open_port SETTING WHAT: open the UDP port of an address in instance.env.
open_port() {
	addr=$(sed -n "s/^$1=//p" instance.env | tail -n 1)
	port=${addr##*:}
	if [ -z "$port" ]; then
		echo "no $1 in instance.env: no port to open for the $2" >&2
		exit 1
	fi
	if ! command -v ufw >/dev/null 2>&1; then
		echo "no ufw on this host: open $port/udp in its firewall yourself"
	elif ufw status | grep -q '^Status: active'; then
		ufw allow "$port/udp" comment "drywire $2" >/dev/null
		echo "ufw: $port/udp open for the $2"
	else
		echo "ufw is installed but inactive: $port/udp is not filtered by it"
	fi
	echo "remember the provider's firewall, if any: $port/udp must be open there too"
}

open_port DRYWIRE_UDP_ADDR relay
# The relay's WebTransport side, for browsers (ADR 0038, 0049).
open_port DRYWIRE_WEB_ADDR "relay for browsers"
