#!/bin/sh
# Format all code in place, component by component.
set -eu

cd "$(dirname "$0")/.."

# Homebrew installs rustup keg-only, outside the PATH; cargo installs land
# in ~/.cargo/bin. Appending both is harmless where they do not exist.
PATH="$PATH:/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin"
root=$(pwd)

if [ -f server/go.mod ]; then
	echo "==> server (gofmt)"
	gofmt -w server
fi

if [ -f "$root/core/Cargo.toml" ]; then
	echo "==> core (cargo fmt)"
	cd "$root/core"
	cargo fmt
fi

if [ -f "$root/desktop/src-tauri/Cargo.toml" ]; then
	echo "==> desktop (cargo fmt)"
	cd "$root/desktop/src-tauri"
	cargo fmt
fi

if [ -f "$root/site/package.json" ]; then
	echo "==> site (Biome)"
	cd "$root/site"
	PATH="$PATH:$HOME/.bun/bin"
	if command -v bun >/dev/null 2>&1; then
		bun install --frozen-lockfile
		bun run format
	else
		echo "note: site skipped; install Bun (https://bun.sh)"
	fi
fi
