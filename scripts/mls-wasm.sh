#!/bin/sh
# Builds the browser's OpenMLS module (core/drywire-mls-wasm, ADR 0052)
# and its JavaScript glue into apps/web/src/mls-wasm/, which git ignores:
# the web client loads it the first time a space's group is needed.
# OpenMLS takes its clock and randomness from the page there, through
# wasm-bindgen, whose command-line tool must match the crate's version.
set -eu
cd "$(dirname "$0")/.."

want=0.2.130
if ! command -v wasm-bindgen >/dev/null 2>&1 || [ "$(wasm-bindgen --version | cut -d' ' -f2)" != "$want" ]; then
	echo "drywire: the web client needs wasm-bindgen $want: cargo install wasm-bindgen-cli --version $want --locked" >&2
	exit 1
fi
(cd core && cargo build -q --release -p drywire-mls-wasm --target wasm32-unknown-unknown)
wasm-bindgen --target web --no-typescript --out-dir apps/web/src/mls-wasm \
	core/target/wasm32-unknown-unknown/release/drywire_mls_wasm.wasm
