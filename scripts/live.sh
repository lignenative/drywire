#!/bin/sh
# A live call on this Mac: a local server, an echo bot, and you.
#
# The script makes a space on a fresh local server, as its owner "alex",
# and invites "sam", an echo bot that sends back everything it hears, with
# an invitation link. You then join as alex with the default microphone
# and speakers. Speak and you hear yourself
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
# Fresh state for every run: this directory belongs to this script.
live="$root/.dev/live"
rm -rf "$live"
mkdir -p "$live"

pids=""
cleanup() {
	# shellcheck disable=SC2086 # word splitting is intended
	[ -n "$pids" ] && kill $pids 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# wait_for PID LOG TEXT WHAT: waits up to 5 s for TEXT in LOG. If PID exits
# first, or the time runs out, prints LOG and stops: going on would hide
# the failure, or join whatever else answers on the port.
wait_for() {
	tries=0
	until grep -q "$3" "$2" 2>/dev/null; do
		if ! kill -0 "$1" 2>/dev/null || [ "$tries" -ge 50 ]; then
			echo "drywire: $4 did not start; its log ($2):" >&2
			sed 's/^/  /' "$2" >&2
			return 1
		fi
		tries=$((tries + 1))
		sleep 0.1
	done
}

: "${DRYWIRE_UDP_ADDR:=127.0.0.1:10050}"
: "${DRYWIRE_HTTP_ADDR:=127.0.0.1:10050}"
DRYWIRE_UDP_ADDR="$DRYWIRE_UDP_ADDR" DRYWIRE_HTTP_ADDR="$DRYWIRE_HTTP_ADDR" \
	DRYWIRE_DB="$live/drywire.db" DRYWIRE_ACCESS_LOG=off DRYWIRE_SPACE_CREATORS='*' \
	"$root/.dev/drywire-server" 2>.dev/live-relay.log &
relay=$!
pids="$pids $relay"
if ! wait_for "$relay" .dev/live-relay.log "drywire-server started" "the server"; then
	echo "A port it needs ($DRYWIRE_UDP_ADDR udp, $DRYWIRE_HTTP_ADDR tcp) is probably" >&2
	echo "taken, often by \`make dev\` or another \`make live\`. Stop that server, or" >&2
	echo "move this one: make live DRYWIRE_UDP_ADDR=127.0.0.1:10058 DRYWIRE_HTTP_ADDR=127.0.0.1:10058" >&2
	exit 1
fi

# Alex makes the space and an invitation; sam joins with the link alone.
"$cli" create-space --home "$live/alex" --instance "http://$DRYWIRE_HTTP_ADDR" \
	--name Live --display-name alex >/dev/null
link=$("$cli" invite --home "$live/alex" --uses 1 2>/dev/null)

"$cli" join --home "$live/sam" --link "$link" --display-name sam --echo --quiet \
	>/dev/null 2>.dev/live-echo.log &
echo_bot=$!
pids="$pids $echo_bot"
wait_for "$echo_bot" .dev/live-echo.log "joined as" "the echo bot" || exit 1

echo "==> joining as alex; speak, and sam echoes you back (Ctrl-C to stop)"
# shellcheck disable=SC2086 # ARGS is a list of options
"$cli" join --home "$live/alex" ${ARGS:-}
