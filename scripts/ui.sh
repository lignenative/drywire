#!/bin/sh
# Run the desktop app's interface alone, in any browser, against a
# simulated call: no Tauri window, no Rust build, no server (ADR 0037).
# Vite listens on 127.0.0.1:10051 (DEV_PORT moves it); open that address.
set -eu

cd "$(dirname "$0")/../desktop"

# Bun installs itself in ~/.bun/bin; appending it is harmless elsewhere.
PATH="$PATH:$HOME/.bun/bin"

if ! command -v bun >/dev/null 2>&1; then
	echo "The interface needs Bun: https://bun.sh" >&2
	exit 1
fi

bun install --frozen-lockfile

port="${DEV_PORT:-10051}"
export DEV_PORT="$port"
echo "Open http://127.0.0.1:$port in a browser: the call there is simulated."
exec bun run dev
