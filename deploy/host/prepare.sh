#!/bin/sh
# Make the host ready to receive Drywire: rsync installed, and /srv/drywire
# owned by the user make deploy logs in as. Runs as root through sudo, read
# from standard input, before anything of Drywire is on the host.
set -eu

if ! command -v rsync >/dev/null 2>&1; then
	echo "installing rsync"
	DEBIAN_FRONTEND=noninteractive apt-get install -y -qq rsync >/dev/null
fi

user=${SUDO_USER:?run it through sudo}
# Only the deploying user reads it: instance.env and the binary live here.
install -d -m 0700 -o "$user" -g "$(id -gn "$user")" /srv/drywire
