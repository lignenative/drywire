#!/bin/sh
# Replay the simulated network traces against the jitter buffer and print
# the latency it adds, its concealment and its drops, per frame size.
# Arguments are passed on: `make bench ARGS="--seed 7"`.
set -eu

cd "$(dirname "$0")/../core"
PATH="$PATH:/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin"
# shellcheck disable=SC2086 # ARGS is split on purpose
cargo run --quiet --release -p drywire-bench -- ${ARGS:-}
