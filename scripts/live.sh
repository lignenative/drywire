#!/bin/sh
# A live call on this Mac: a local relay, an echo bot, and you.
#
# You join as "alex" with the default microphone and speakers; "sam" is an
# echo bot that sends back everything it hears. Speak and you hear yourself
# come back through the relay twice, so the delay you hear is about two
# mouth-to-ear trips minus one capture and one playback. Use headphones:
# with speakers, the bot's echo reaches the microphone again and loops.
#
# macOS asks once for microphone access for your terminal. Extra arguments
# go to drywire-cli join, for example: make live ARGS="--frame 2.5".
# Ctrl-C stops everything.
set -eu

cd "$(dirname "$0")/.."
PATH="$PATH:/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin"
root=$(pwd)
mkdir -p .dev

echo "==> building the relay and drywire-cli (release)"
(cd server && go build -o "$root/.dev/drywire-server" ./cmd/drywire-server)
(cd core && cargo build --release -p drywire-cli)
cli="$root/core/target/release/drywire-cli"
dev="$root/core/drywire-cli/dev"

pids=""
cleanup() {
	# shellcheck disable=SC2086 # word splitting is intended
	[ -n "$pids" ] && kill $pids 2>/dev/null || true
}
trap cleanup EXIT INT TERM

: "${DRYWIRE_UDP_ADDR:=127.0.0.1:10050}"
DRYWIRE_ROOMS=server/rooms.dev.json DRYWIRE_UDP_ADDR="$DRYWIRE_UDP_ADDR" \
	"$root/.dev/drywire-server" 2>.dev/live-relay.log &
pids="$pids $!"
sleep 0.5

"$cli" join --config "$dev/sam.json" --relay "$DRYWIRE_UDP_ADDR" --echo --quiet \
	>/dev/null 2>.dev/live-echo.log &
pids="$pids $!"
sleep 0.5

echo "==> joining as alex; speak, and sam echoes you back (Ctrl-C to stop)"
# shellcheck disable=SC2086 # ARGS is a list of options
"$cli" join --config "$dev/alex.json" --relay "$DRYWIRE_UDP_ADDR" ${ARGS:-}
