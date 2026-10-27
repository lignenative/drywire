#!/bin/sh
# Format all code in place, component by component.
set -eu

cd "$(dirname "$0")/.."
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
