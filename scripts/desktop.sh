#!/bin/sh
# Run the desktop app in development: Vite on 127.0.0.1:10051 (DEV_PORT
# moves it), and the Tauri window, rebuilt when the Rust side changes.
# Talk to a local server with `make dev` in another terminal. See
# desktop/README.md.
set -eu

cd "$(dirname "$0")/../desktop"

# Homebrew installs rustup keg-only, outside the PATH; Bun installs itself
# in ~/.bun/bin. Appending them is harmless where they do not exist.
PATH="$PATH:/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$HOME/.bun/bin"

if ! command -v bun >/dev/null 2>&1; then
	echo "The desktop app needs Bun: https://bun.sh" >&2
	exit 1
fi

bun install --frozen-lockfile

# Tauri loads the interface from the port Vite listens on; both follow
# DEV_PORT, and Vite refuses to start rather than take another port.
port="${DEV_PORT:-10051}"
export DEV_PORT="$port"
exec bun run tauri dev --config "{\"build\":{\"devUrl\":\"http://127.0.0.1:$port\"}}" "$@"
