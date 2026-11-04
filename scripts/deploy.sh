#!/bin/sh
# Deploy Drywire to HOST. Idempotent: run it again after any change.
#
# 1. Build drywire-server and drywire-static here, static Linux binaries
#    (CGO_ENABLED=0) for the host's processor, and the web client
#    (apps/web, with its WebAssembly module) that drywire-static serves.
# 2. Send them over rsync to /srv/drywire, with deploy/'s Dockerfiles,
#    Compose files, host scripts and instance.env, plus two generated
#    files: .env, which tells Docker Compose how the server meets its
#    reverse proxy, and route.caddy, the proxy's route to the instance:
#    /v1/… to the control plane, the rest to the web client (ADR 0049).
# 3. Open the relay's UDP ports in the host's firewall (ufw).
# 4. Build the images there (from scratch: nothing is downloaded) and
#    start them with Docker Compose, then wait for their health checks.
# 5. If ROUTE_PUBLISH_CMD is set in .env, hand route.caddy to that command
#    on the host.
#
# The host never pulls from GitHub or a registry: this machine is the
# reference. See deploy/README.md for what the host needs first.
set -eu

cd "$(dirname "$0")/.."
. scripts/host.sh

[ -f deploy/instance.env ] ||
	fail "deploy/instance.env is missing: cp deploy/instance.env.example deploy/instance.env, then fill it in"
public_url=$(instance_setting DRYWIRE_PUBLIC_URL)
case $public_url in
https://*) ;;
*) fail "DRYWIRE_PUBLIC_URL in deploy/instance.env must be the https:// address of your reverse proxy" ;;
esac
case $public_url in
*example.org*) fail "deploy/instance.env still has the example's addresses: set your own" ;;
esac
public_host=${public_url#https://}
public_host=${public_host%%/*}

# Both addresses must be written down: the server's own defaults are the
# development ones (127.0.0.1:10050), which a real host must not get by
# accident.
http_addr=$(instance_setting DRYWIRE_HTTP_ADDR)
relay_addr=$(instance_setting DRYWIRE_UDP_ADDR)
if [ -z "$http_addr" ] || [ -z "$relay_addr" ]; then
	fail "set DRYWIRE_HTTP_ADDR and DRYWIRE_UDP_ADDR in deploy/instance.env (see instance.env.example)"
fi
# The relay's port (":30050" binds every address).
relay_port=${relay_addr##*:}
case $(instance_setting DRYWIRE_PUBLIC_RELAY) in
*:"$relay_port") ;;
*) fail "DRYWIRE_PUBLIC_RELAY must end with the port of DRYWIRE_UDP_ADDR, :$relay_port" ;;
esac
# The relay's WebTransport side, for the web client the instance serves.
web_addr=$(instance_setting DRYWIRE_WEB_ADDR)
[ -n "$web_addr" ] ||
	fail "set DRYWIRE_WEB_ADDR and DRYWIRE_PUBLIC_WEB_RELAY in deploy/instance.env, for browsers (see instance.env.example)"
web_port=${web_addr##*:}
[ "$web_port" != "$relay_port" ] ||
	fail "DRYWIRE_WEB_ADDR needs a UDP port of its own, not the relay's $relay_port (ADR 0038)"
case $(instance_setting DRYWIRE_PUBLIC_WEB_RELAY) in
*:"$web_port") ;;
*) fail "DRYWIRE_PUBLIC_WEB_RELAY must end with the port of DRYWIRE_WEB_ADDR, :$web_port" ;;
esac

proxy_network=${DRYWIRE_PROXY_NETWORK:-}
case $proxy_network in
*[!A-Za-z0-9_.-]*) fail "DRYWIRE_PROXY_NETWORK is not a Docker network name: $proxy_network" ;;
esac

echo "==> $HOST: checking Docker and preparing $dir"
ssh_host 'docker compose version >/dev/null' ||
	fail "Docker Compose does not answer on $HOST for this user (see deploy/README.md, \"The host\")"
ssh_host 'sudo -n sh -s' <deploy/host/prepare.sh

case $(ssh_host 'uname -m') in
x86_64) arch=amd64 ;;
aarch64 | arm64) arch=arm64 ;;
*) fail "$HOST runs on a processor Drywire is not built for: $(ssh_host 'uname -m')" ;;
esac

version=$(git describe --tags --always --dirty)
stage=.dev/deploy
echo "==> building drywire-server and drywire-static $version for linux/$arch"
rm -rf "$stage"
mkdir -p "$stage"
(cd server && GOOS=linux GOARCH=$arch CGO_ENABLED=0 \
	go build -trimpath -ldflags "-s -w -X main.version=$version" \
	-o "../$stage/drywire-server" ./cmd/drywire-server &&
	GOOS=linux GOARCH=$arch CGO_ENABLED=0 \
		go build -trimpath -ldflags "-s -w" -o "../$stage/drywire-static" ./cmd/drywire-static)

echo "==> building the web client"
PATH="$PATH:/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$HOME/.bun/bin"
command -v bun >/dev/null 2>&1 || fail "the web client needs Bun (https://bun.sh)"
rustup target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$' ||
	fail "the web client needs: rustup target add wasm32-unknown-unknown"
(cd core && cargo build -q --release -p drywire-wasm --target wasm32-unknown-unknown)
mkdir -p apps/web/public
cp core/target/wasm32-unknown-unknown/release/drywire_wasm.wasm apps/web/public/drywire.wasm
# OpenMLS for the spaces' groups, loaded when one is needed (ADR 0052).
scripts/mls-wasm.sh
# Errors only: the UI libraries' "use client" directives warn by the hundred.
(cd apps/web && bun install --frozen-lockfile >/dev/null &&
	bun run build --logLevel error --outDir "../../$stage/web" --emptyOutDir >/dev/null)

cp -R deploy/Dockerfile deploy/static.Dockerfile deploy/compose.yaml \
	deploy/compose.proxy.yaml deploy/instance.env deploy/host "$stage/"

# .env in the project's directory is where Docker Compose finds which
# files to use and the values compose.proxy.yaml needs; every docker
# compose command on the host reads it.
if [ -z "$proxy_network" ]; then
	echo "==> the reverse proxy is on the host: the server uses the host's network"
	echo "COMPOSE_FILE=compose.yaml" >"$stage/.env"
	upstream=$http_addr
	# compose.yaml's address for drywire-static.
	front=127.0.0.1:10052
else
	subnet=$(ssh_host "docker network inspect --format '{{range .IPAM.Config}}{{.Subnet}} {{end}}' $proxy_network") ||
		fail "no Docker network named $proxy_network on $HOST (DRYWIRE_PROXY_NETWORK)"
	# One subnet, or an IPv4 and an IPv6 one: comma-separated for the server.
	subnet=$(printf '%s' "$subnet" | sed -e 's/ *$//' -e 's/ /,/g')
	[ -n "$subnet" ] || fail "the Docker network $proxy_network has no subnet"
	echo "==> the reverse proxy is in the Docker network $proxy_network ($subnet): the server joins it"
	cat >"$stage/.env" <<ENV
COMPOSE_FILE=compose.yaml:compose.proxy.yaml
DRYWIRE_PROXY_NETWORK=$proxy_network
DRYWIRE_PROXY_SUBNET=$subnet
DRYWIRE_RELAY_PORT=$relay_port
DRYWIRE_WEB_RELAY_PORT=$web_port
ENV
	# compose.proxy.yaml sets both addresses in the network.
	upstream=drywire:10050
	front=drywire-web:10052
fi

# The proxy's route to the instance, as a Caddy site block: the control
# plane's API under /v1/, the web client's files everywhere else.
cat >"$stage/route.caddy" <<ROUTE
$public_host {
	handle /v1/* {
		reverse_proxy $upstream {
			# While the server restarts during a deploy, wait for it
			# instead of failing requests at once.
			lb_try_duration 10s
			lb_try_interval 250ms
			# Hand presence events (text/event-stream) on as they come.
			flush_interval -1
		}
	}
	handle {
		encode zstd gzip
		reverse_proxy $front {
			lb_try_duration 10s
			lb_try_interval 250ms
		}
	}
}
ROUTE

echo "==> sending it to $HOST:$dir"
# --delete keeps the host's copy exactly this one. The data is not in
# $dir but in a Docker volume, which rsync never touches. $dir is only
# open to its owner (host/prepare.sh), whatever the files' own modes.
rsync -rlt --delete -e "ssh $ssh_opts" "$stage/" "$HOST:$dir/"

echo "==> opening the relay's ports"
ssh_host "sudo -n sh $dir/host/setup.sh"

echo "==> starting it"
on_host "sh host/up.sh"

if [ -n "${ROUTE_PUBLISH_CMD:-}" ]; then
	echo "==> publishing the route to $public_host"
	ssh_host "$ROUTE_PUBLISH_CMD" <"$stage/route.caddy" ||
		fail "ROUTE_PUBLISH_CMD failed on $HOST; the route is in $dir/route.caddy"
fi

echo "==> checking $public_url from here"
if curl -fsS --max-time 10 "$public_url/v1/version" &&
	curl -fsS --max-time 10 -o /dev/null "$public_url/"; then
	echo
	echo "Deployed, with the web client at $public_url/."
	echo "Next: make status, and deploy/README.md for the first space."
else
	echo "The server runs on $HOST, but $public_url does not answer yet." >&2
	echo "Set up the reverse proxy and DNS: deploy/README.md, \"Reverse proxy\"; the" >&2
	echo "route it needs is in $dir/route.caddy on the host." >&2
	exit 1
fi
