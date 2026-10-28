#!/bin/sh
# Deploy Drywire to HOST. Idempotent: run it again after any change.
#
# 1. Build drywire-server here, a static Linux binary (CGO_ENABLED=0) for
#    the host's processor.
# 2. Send it over rsync to /srv/drywire, with deploy/'s Dockerfile,
#    compose.yaml, host scripts, instance.env and, if you wrote one,
#    compose.override.yaml.
# 3. Open the relay's UDP port in the host's firewall (ufw).
# 4. Build the image there (from scratch: nothing is downloaded) and start
#    it with Docker Compose, then wait for its health check.
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
echo "==> building drywire-server $version for linux/$arch"
rm -rf "$stage"
mkdir -p "$stage"
(cd server && GOOS=linux GOARCH=$arch CGO_ENABLED=0 \
	go build -trimpath -ldflags "-s -w -X main.version=$version" \
	-o "../$stage/drywire-server" ./cmd/drywire-server)
cp -R deploy/Dockerfile deploy/compose.yaml deploy/instance.env deploy/host "$stage/"
# The operator's own adjustments, such as a proxy in a Docker network
# (deploy/compose.override.example.yaml).
[ ! -f deploy/compose.override.yaml ] || cp deploy/compose.override.yaml "$stage/"

echo "==> sending it to $HOST:$dir"
# --delete keeps the host's copy exactly this one. The data is not in
# $dir but in a Docker volume, which rsync never touches. $dir is only
# open to its owner (host/prepare.sh), whatever the files' own modes.
rsync -rlt --delete -e "ssh $ssh_opts" "$stage/" "$HOST:$dir/"

echo "==> opening the relay port"
ssh_host "sudo -n sh $dir/host/setup.sh"

echo "==> starting it"
on_host "sh host/up.sh"

echo "==> checking $public_url from here"
if curl -fsS --max-time 10 "$public_url/v1/version"; then
	echo
	echo "Deployed. Next: make status, and deploy/README.md for the first space."
else
	echo "The server runs on $HOST, but $public_url does not answer yet." >&2
	echo "Set up the reverse proxy and DNS: deploy/README.md, \"Reverse proxy\"." >&2
	exit 1
fi
