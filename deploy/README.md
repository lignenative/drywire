# deploy

How to run a real Drywire instance: the relay and the control plane on a
Linux server, the control plane behind your reverse proxy's TLS, operated
from your own machine with `make`.

```text
 your machine                         the host (HOST, an SSH alias)
 ────────────                         ─────────────────────────────
 make deploy ──build, rsync──▶ /srv/drywire ──docker compose──▶ drywire-server
 make backup ◀──snapshot, encrypted here with age──               │
                                                                    │ host network
 clients ──HTTPS 443──▶ reverse proxy (Caddy, nginx) ──▶ 127.0.0.1:10050/tcp
 clients ──UDP──────────────────────────────────────────▶ :30050/udp (relay)
```

- Your machine is the reference. `make deploy` builds the server here and
  sends it; the host never pulls from GitHub or a registry.
- Drywire owns `/srv/drywire` and one Docker volume on the host, and
  touches nothing else, so it can share a server with other services.
- The container runs on the host's own network: nothing sits between the
  network and the relay, and the reverse proxy reaches the control plane
  on the host's `127.0.0.1` ([ADR 0026](../docs/adr/0026-run-the-server-in-a-scratch-image-on-the-host-network.md)).

## What you need

On **the host**:

- Linux on x86-64 or ARM64, reachable over SSH with a key.
- **Docker Engine 25** or newer with the Compose plugin, usable by the user
  you log in as (in the `docker` group).
- **sudo without a password** for that user: `make deploy` uses it to
  create `/srv/drywire` and to open the relay port in ufw (the scripts are
  [`host/prepare.sh`](host/prepare.sh) and [`host/setup.sh`](host/setup.sh);
  read them first).
- A **DNS name** for the instance, such as `voice.example.org`, and a
  **reverse proxy** on the same host that serves it over HTTPS (below).

On **your machine**: Go (the version in `server/go.mod`), rsync, curl, and
[age](https://age-encryption.org) for the backups (`brew install age`, or
`apt install age`). `drywire-cli` to create the first space: see
[`core/drywire-cli/README.md`](../core/drywire-cli/README.md).

## First deployment

1. **Name the host.** In `~/.ssh/config`, an alias for the server; then
   in `.env` at the repository root (copied from `.env.example`, never
   committed):

   ```sh
   HOST=voice-host
   ```

   `.env` also says where the reverse proxy is (`DRYWIRE_PROXY_NETWORK`)
   and how to publish its route (`ROUTE_PUBLISH_CMD`), below.

2. **Describe the instance.** Copy
   [`instance.env.example`](instance.env.example) to `deploy/instance.env`
   (never committed) and set your own addresses:

   ```sh
   DRYWIRE_PUBLIC_URL=https://voice.example.org
   DRYWIRE_PUBLIC_RELAY=voice.example.org:30050
   DRYWIRE_SPACE_CREATORS=<your key>
   ```

   Your key is what `drywire-cli identity` prints on the device you will
   create the first space from. Only the keys listed here can create
   spaces; anyone you invite can still join.

3. **Set up the reverse proxy and the firewall** on the host (the next two
   sections).

4. **Deploy:**

   ```sh
   make deploy
   ```

   It builds, sends, opens the relay port if the host uses ufw, starts the
   server, waits for its health check, and ends by asking the public URL
   for `/v1/version`. Run it again after any change: it is idempotent, and
   restarts the server only when something changed.

5. **Check it:** `make status` shows the container, then the version,
   uptime and active rooms as clients see them. `make logs` follows the
   logs.

## Reverse proxy

The control plane speaks plain HTTP on `127.0.0.1:10050` only; the proxy
terminates TLS in front of it. Three things matter:

- **Same host, loopback.** The proxy must reach `127.0.0.1:10050` of the
  host. A proxy installed on the host does. A proxy that runs in a
  container on a Docker network does not (its `127.0.0.1` is its own):
  see [A proxy in a Docker network](#a-proxy-in-a-docker-network).
- **The client's address** goes in `X-Forwarded-For`. With
  `DRYWIRE_TRUST_PROXY=1` (in `instance.env.example`), the server takes the
  last entry of that header, the one your proxy wrote, for its rate limits
  and access log. Without it, every client looks like `127.0.0.1` and they
  share one rate limit.
- **Event streams.** Presence comes as a long-lived
  `text/event-stream` response: the proxy must pass it through without
  buffering and without a short read timeout.

### Caddy

Caddy gets and renews the certificate itself, and sets `X-Forwarded-For`
(dropping any value a client sent). `make deploy` writes this route for
your instance to `/srv/drywire/route.caddy` on the host:

```caddyfile
voice.example.org {
	reverse_proxy 127.0.0.1:10050 {
		lb_try_duration 10s
		lb_try_interval 250ms
		flush_interval -1
	}
}
```

- `flush_interval -1` hands each response on as soon as the server writes
  it. Presence (who joins, leaves or renames) is a `text/event-stream`
  response that stays open for the whole call; a proxy that buffers it
  holds the events back until its buffer fills, so people would appear in
  a room long after they joined. Recent Caddy versions already flush
  event streams at once, but saying it keeps presence live whatever the
  version.
- `lb_try_duration` and `lb_try_interval` make Caddy retry for up to
  10 seconds while the server restarts during a deploy, instead of
  answering 502 at once.

**Publishing the route by itself.** If the host has a command that
installs a route read on its standard input (one that checks the site
block, reloads the proxy, and puts the old route back if that fails),
name it in `.env`:

```sh
ROUTE_PUBLISH_CMD=/usr/local/bin/publish-route drywire
```

`make deploy` then pipes `route.caddy` into it over SSH once the server
is healthy. Left empty, you add the route to your Caddyfile yourself.

### nginx

With a certificate from your ACME client (certbot, acme.sh…):

```nginx
server {
    listen 443 ssl;
    listen [::]:443 ssl;
    server_name voice.example.org;

    ssl_certificate     /etc/letsencrypt/live/voice.example.org/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/voice.example.org/privkey.pem;
    ssl_protocols       TLSv1.3;

    location / {
        proxy_pass http://127.0.0.1:10050;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        # Replace, never append, what the client sent.
        proxy_set_header X-Forwarded-For $remote_addr;
        # Presence events: pass them on as they come, keep the stream open.
        proxy_buffering off;
        proxy_read_timeout 1h;
    }
}
```

### A proxy in a Docker network

If your reverse proxy runs in a container on a Docker network that the
applications it serves join (a shared "edge" or "proxy" network), name
that network in `.env`:

```sh
DRYWIRE_PROXY_NETWORK=edge
```

`make deploy` then looks up the network's subnet on the host and adds
[`compose.proxy.yaml`](compose.proxy.yaml) to the deployment, through a
generated `/srv/drywire/.env` that Docker Compose reads by itself
([ADR 0027](../docs/adr/0027-let-a-proxy-on-a-private-network-reach-the-control-plane.md)):

- the server leaves the host's network and joins the proxy's, where the
  proxy reaches the control plane as `drywire:10050` (the route in
  `route.caddy` says so); no host port is published for it;
- it serves plain HTTP there, accepts connections from that subnet and
  loopback only, and believes `X-Forwarded-For` from them; it refuses to
  start if the subnet is not a private one;
- only the relay's UDP port is published on the host.

A port Docker publishes goes around ufw: the relay is reachable whatever
ufw says, and the rule `make deploy` adds keeps `ufw status` truthful.
The audio then passes Docker's port forwarding, a kernel NAT rule that
keeps IPv4 clients' addresses. Every container on the proxy's network
can reach the control plane in plain HTTP, so keep that network to
applications you trust.

## Firewall

Clients need two ports on the host: the proxy's **443/tcp** (and 80/tcp if
your ACME client uses the HTTP challenge), and the relay's **30050/udp**,
or whatever port `DRYWIRE_UDP_ADDR` names
([ADR 0028](../docs/adr/0028-serve-the-relay-on-udp-30050-in-production.md):
10050 is the development port). The control plane's 10050/tcp stays
closed: nothing outside the host, or the proxy's Docker network, reaches
it.

With **ufw**, `make deploy` opens the relay port itself (and only that);
the equivalent by hand:

```sh
sudo ufw allow 443/tcp comment 'https'
sudo ufw allow 30050/udp comment 'drywire relay'
```

With **nftables**, in the `input` chain of your `inet filter` table:

```nft
tcp dport 443 accept
udp dport 30050 accept
```

On the host's network, these rules really decide what is reachable. On a
proxy's Docker network (above), Docker publishes the relay's port, which
goes around them.

If your **provider** has a network firewall in front of the server (a
"security group", an "edge firewall"…), open the same ports there too. A
relay port closed there shows as clients that join a room but never hear
anyone.

## The first space and invitation

From the device whose key is in `DRYWIRE_SPACE_CREATORS`:

```sh
drywire-cli create-space --instance https://voice.example.org --name Team --display-name alex
drywire-cli invite --uses 5 --days 7
```

`invite` prints a link (`https://drywire.gg/i#…`). The part after `#`
holds the space's key: the server never sees it, so share the link
privately. Each teammate then joins and talks with:

```sh
drywire-cli join --link '<the link>' --display-name sam
```

Later, `drywire-cli join` alone takes them back to the space. `make status`
shows the rooms in use and the packet counters.

## Updating

Pull or change the code, then `make deploy`. The server restarts, which
ends calls in progress for a moment: deploy when nobody is playing. The
database schema migrates by itself at start; a server never opens a
database written by a newer one.

## Backups

`make backup` copies a snapshot of the database to `backups/` on your
machine (ignored by git). The server takes it while it runs, so nobody is
disconnected, and it is encrypted here with age as it arrives: the plain
copy never rests on a disk, on the host or on your machine.

What you provide, in `.env`:

```sh
# Make the key pair once; keep the file off the server, and a copy of it
# somewhere safe: without it, the backups cannot be read.
age-keygen -o ~/.config/drywire/backup-key.txt
```

```sh
BACKUP_RECIPIENTS=age1...            # the public key it printed; several, separated by spaces
BACKUP_IDENTITY=/Users/you/.config/drywire/backup-key.txt
```

- **Retention:** 6 months at most (spec section 9). Every `make backup`
  deletes the backups in `backups/` older than 180 days. Copies you make
  elsewhere are yours to delete on time.
- **What is in it:** identities (public keys and display names), spaces,
  rooms, memberships, bans and invitation records. No audio, no token, no
  invitation code (only their hashes), no IP address.
- **What is not:** the deletion ledger. It stays on the host, so that a
  restore can delete again what was deleted after the backup
  ([ADR 0019](../docs/adr/0019-replay-a-deletion-ledger-at-every-start.md)).

Run it by hand for now, after changes you would not want to lose, or from
your machine's scheduler.

## Restore

```sh
make restore                                   # the newest backup in backups/
make restore FILE=backups/drywire-20261028-120000.db.age
```

It asks before doing anything. Then the server stops; the backup is
decrypted on your machine and sent to the host, where `drywire-server
restore` checks it (a SQLite database, intact, not from a newer server),
replaces the database, and **replays the deletion ledger** on it, saying
how many deletions it applied again: an identity, a space, a membership or
an invitation deleted since the backup stays deleted (spec section 9). The
server then starts again. A backup that does not decrypt, or is damaged,
changes nothing: the server starts again on its current database.

Bans are not replayed: bans made since the backup are lost
([ADR 0019](../docs/adr/0019-replay-a-deletion-ledger-at-every-start.md)).
After a restore, revoke the invitations that matter.

**Moving to a new host:** deploy there, copy the old host's deletion
ledger into the new volume while the server is stopped, then restore. On
the old host, `docker compose cp drywire:/data/drywire.deletions -` writes
it (as a tar stream); on the new one, `docker compose stop drywire`, then
`docker compose cp - drywire:/data/` reads it back. Without the ledger,
`restore` warns that deletions since the backup cannot be replayed.

Practise a restore now and then on a test server: `make deploy` and
`make restore` with another `HOST`.

## Logs

The server writes two kinds of lines to standard output: its application
log, which never holds an address, a token, a key or anything a user
wrote, and the access log (JSON lines: client address, method, route
pattern, status, duration; spec section 9). Docker keeps them in rotating
files, 3 × 10 MB at most, deleted when the container is recreated.

The access log may be kept **one year at most**. The size limit keeps it
far shorter on an instance in use; on a very quiet one, check now and then
that nothing older than a year remains (`make logs` shows the oldest lines
first), or set `DRYWIRE_ACCESS_LOG` to a file you rotate yourself.

## Without Docker

[`drywire.service`](drywire.service) runs the server under systemd, as
its own system user, with the same `instance.env` and the same
confinement as the container (the install steps are at the top of the
file). Build the binary on your machine and copy it over:

```sh
cd server && GOOS=linux GOARCH=amd64 CGO_ENABLED=0 go build -trimpath -o drywire-server ./cmd/drywire-server
```

The `make` targets drive the Docker deployment only. With systemd:

```sh
ssh voice-host 'sudo systemctl status drywire'
ssh voice-host 'journalctl -u drywire -f'
ssh voice-host 'sudo -u drywire DRYWIRE_DB=/var/lib/drywire/drywire.db /usr/local/bin/drywire-server backup' \
  | age -r age1... -o backups/drywire-$(date -u +%Y%m%d-%H%M%S).db.age
ssh voice-host 'sudo systemctl stop drywire'
age -d -i ~/.config/drywire/backup-key.txt backups/<file>.db.age \
  | ssh voice-host 'sudo -u drywire DRYWIRE_DB=/var/lib/drywire/drywire.db DRYWIRE_LEDGER=/var/lib/drywire/drywire.deletions /usr/local/bin/drywire-server restore'
ssh voice-host 'sudo systemctl start drywire'
```

The journal keeps the access log; set its retention to a year at most
(`MaxRetentionSec=1year` in `journald.conf`), or less.

## Try it locally

Docker Desktop has no host network by default, so
[`compose.local.yaml`](compose.local.yaml) runs the same image with a
Caddy container playing the host and its reverse proxy, on this machine's
`127.0.0.1:10050` (stop `make dev` first: same ports):

```sh
mkdir -p .dev/try && cp deploy/Dockerfile deploy/compose.yaml deploy/compose.local.yaml .dev/try/
(cd server && GOOS=linux GOARCH=arm64 CGO_ENABLED=0 go build -o ../.dev/try/drywire-server ./cmd/drywire-server)
cd .dev/try && docker compose -f compose.yaml -f compose.local.yaml up --build -d
curl http://127.0.0.1:10050/v1/status
```

(`GOARCH=amd64` on an Intel machine.) Then use `drywire-cli` with
`--instance http://127.0.0.1:10050` as with `make dev`, and finish with
`docker compose -f compose.yaml -f compose.local.yaml down -v`.

## Files

| File | What it does |
| --- | --- |
| `Dockerfile` | The release image: the prebuilt static binary in `scratch`, unprivileged, state in `/data` |
| `compose.yaml` | The service on the host network, its volume, health check and log rotation |
| `instance.env.example` | An instance's settings; copied to `instance.env` |
| `compose.proxy.yaml` | Added by `make deploy` for a reverse proxy in a Docker network (`DRYWIRE_PROXY_NETWORK`) |
| `compose.local.yaml` | Trying the image on Docker Desktop |
| `drywire.service` | The systemd unit, for a host without Docker |
| `.env`, `route.caddy` | Not in the repository: written by `make deploy` on the host, for Docker Compose and the reverse proxy |
| `host/` | What `make` runs on the host: `prepare.sh` and `setup.sh` (as root), `up.sh`, `wait.sh`, `status.sh`, `backup.sh`, `restore.sh` |

The scripts on your side are in [`../scripts/`](../scripts/):
`deploy.sh`, `status.sh`, `logs.sh`, `backup.sh`, `restore.sh`, and
`host.sh`, which they share.
