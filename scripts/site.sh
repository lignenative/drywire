#!/bin/sh
# The project site (site/, Astro, ADR 0042). One command per make target:
#
#   scripts/site.sh dev       dev server with live reload, 127.0.0.1:10053
#   scripts/site.sh build     the static site into site/dist/
#   scripts/site.sh preview   build, then serve site/dist/ on 127.0.0.1:10052
#   scripts/site.sh check     type-check (astro check), then lint (Biome)
#   scripts/site.sh stop      stop whatever listens on those two ports
#
# SITE_DEV_PORT and SITE_PORT move the ports; astro.config.mjs reads the
# same variables, and a busy port is an error, never a silent move.
set -eu

cd "$(dirname "$0")/../site"

# Bun installs itself in ~/.bun/bin; appending it is harmless elsewhere.
PATH="$PATH:$HOME/.bun/bin"

dev_port="${SITE_DEV_PORT:-10053}"
preview_port="${SITE_PORT:-10052}"
export SITE_DEV_PORT="$dev_port" SITE_PORT="$preview_port"

need_bun() {
	if ! command -v bun >/dev/null 2>&1; then
		echo "The site needs Bun: https://bun.sh" >&2
		exit 1
	fi
	bun install --frozen-lockfile
}

case "${1:-}" in
dev)
	need_bun
	echo "The site: http://127.0.0.1:$dev_port/ (the invitation page is /i)"
	exec bun run dev
	;;
build)
	need_bun
	exec bun run build
	;;
preview)
	need_bun
	bun run build
	# The built pages carry their security policy, which the dev server's
	# pages cannot: check the invitation page here.
	echo "The built site: http://127.0.0.1:$preview_port/i"
	exec bun run preview
	;;
check)
	need_bun
	bun run check
	exec bun run lint
	;;
stop)
	for port in "$dev_port" "$preview_port"; do
		pids=$(lsof -ti "tcp:$port" -sTCP:LISTEN 2>/dev/null || true)
		if [ -n "$pids" ]; then
			# shellcheck disable=SC2086 # one pid per word
			kill $pids && echo "Stopped port $port"
		fi
	done
	;;
*)
	echo "usage: scripts/site.sh dev|build|preview|check|stop" >&2
	exit 2
	;;
esac
