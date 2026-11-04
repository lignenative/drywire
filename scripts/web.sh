#!/bin/sh
# The web client against a local server and an echo bot, on this Mac.
#
# The script builds the server, the command-line client and the web
# client's WebAssembly module; starts a fresh server with its WebTransport
# side for browsers (127.0.0.1:10058/udp); makes a space as "alex" and
# invites "sam", an echo bot that sends back everything it hears; makes a
# second invitation for the browser, and serves the web client with Vite on
# 127.0.0.1:10057. Open the address it prints, which carries the
# invitation in its fragment: the web client offers to join the space,
# then join its room, allow the microphone, and hear yourself back from
# sam about a second later. Ctrl-C stops everything.
set -eu

cd "$(dirname "$0")/.."
PATH="$PATH:/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$HOME/.bun/bin"
root=$(pwd)
mkdir -p .dev

if ! rustup target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$'; then
	echo "drywire: the web client needs: rustup target add wasm32-unknown-unknown" >&2
	exit 1
fi

echo "==> building the server, drywire and the WebAssembly module (release)"
(cd server && go build -o "$root/.dev/drywire-server" ./cmd/drywire-server)
(cd core && cargo build --release -p drywire-cli &&
	cargo build --release -p drywire-wasm --target wasm32-unknown-unknown)
mkdir -p apps/web/public
cp core/target/wasm32-unknown-unknown/release/drywire_wasm.wasm apps/web/public/drywire.wasm
# OpenMLS for the spaces' groups, loaded when one is needed (ADR 0052).
scripts/mls-wasm.sh
(cd apps/web && bun install --frozen-lockfile >/dev/null)
cli="$root/core/target/release/drywire"
# Fresh state for every run: this directory belongs to this script.
state="$root/.dev/web"
rm -rf "$state"
mkdir -p "$state"

pids=""
cleanup() {
	# shellcheck disable=SC2086 # word splitting is intended
	[ -n "$pids" ] && kill $pids 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# wait_for PID LOG TEXT WHAT: as in live.sh.
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
: "${DRYWIRE_WEB_ADDR:=127.0.0.1:10058}"
: "${WEB_PORT:=10057}"
export DRYWIRE_HTTP_ADDR WEB_PORT
DRYWIRE_UDP_ADDR="$DRYWIRE_UDP_ADDR" DRYWIRE_WEB_ADDR="$DRYWIRE_WEB_ADDR" \
	DRYWIRE_DB="$state/drywire.db" DRYWIRE_ACCESS_LOG=off DRYWIRE_SPACE_CREATORS='*' \
	DRYWIRE_MAIL_DIR="$root/.dev/mail" \
	"$root/.dev/drywire-server" 2>.dev/web-server.log &
pids="$pids $!"
wait_for "$!" .dev/web-server.log "drywire-server started" "the server" || {
	echo "A port it needs ($DRYWIRE_UDP_ADDR udp, $DRYWIRE_HTTP_ADDR tcp, $DRYWIRE_WEB_ADDR udp)" >&2
	echo "is probably taken, often by \`make dev\` or \`make live\`." >&2
	exit 1
}

"$cli" create-space --home "$state/alex" --instance "http://$DRYWIRE_HTTP_ADDR" \
	--name Web --display-name alex >/dev/null
bot_link=$("$cli" invite --home "$state/alex" --uses 1 2>/dev/null)
# A few uses, so the page can be reloaded and run again.
browser_link=$("$cli" invite --home "$state/alex" --uses 5 2>/dev/null)

"$cli" join --home "$state/sam" --link "$bot_link" --display-name sam --echo --quiet \
	>/dev/null 2>.dev/web-echo.log &
pids="$pids $!"
wait_for "$!" .dev/web-echo.log "joined as" "the echo bot" || exit 1

# The invitation goes in the fragment, which the browser never sends.
echo "==> open http://127.0.0.1:$WEB_PORT/#${browser_link#*#} (Ctrl-C to stop)"
cd apps/web
bun --bun vite
