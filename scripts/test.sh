#!/bin/sh
# Run every test suite that exists in the repository. Components are added
# phase by phase, so each one is tested only once its directory is there.
set -eu

cd "$(dirname "$0")/.."

# Homebrew installs rustup keg-only, outside the PATH; cargo installs land
# in ~/.cargo/bin. Appending both is harmless where they do not exist.
PATH="$PATH:/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin"
root=$(pwd)

if [ -f server/go.mod ]; then
	echo "==> server (Go)"
	cd "$root/server"
	unformatted=$(gofmt -l .)
	if [ -n "$unformatted" ]; then
		echo "These files need gofmt (run make fmt):" >&2
		echo "$unformatted" >&2
		exit 1
	fi
	go vet ./...
	go test -race ./...
fi

if [ -f "$root/core/Cargo.toml" ]; then
	echo "==> core (Rust)"
	cd "$root/core"
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test
fi
