# server

`drywire-server`, the Go side of Drywire: the UDP relay that copies audio
packets between the members of a room, and the control plane, an HTTP API
for identities, spaces, rooms and invitations kept in SQLite. Standard
library only, plus `modernc.org/sqlite` (pure Go, so the binary stays
static), and `quic-go` with `webtransport-go` for browsers.

## Run it

From the repository root:

```sh
make dev
```

This serves `127.0.0.1:10050`, UDP for the relay and TCP for the control
plane, and `127.0.0.1:10058` UDP for browsers (WebTransport), with its
database in `.dev/drywire.db` and anyone allowed to create spaces:

```sh
curl http://127.0.0.1:10050/healthz      # ok
curl http://127.0.0.1:10050/v1/version   # server and protocol versions
curl http://127.0.0.1:10050/v1/status    # uptime, rooms, packet counters
```

| Variable | Default | Meaning |
| --- | --- | --- |
| `DRYWIRE_UDP_ADDR` | `127.0.0.1:10050` | Relay address |
| `DRYWIRE_HTTP_ADDR` | `127.0.0.1:10050` | Control plane address |
| `DRYWIRE_PUBLIC_URL` | `http://` + `DRYWIRE_HTTP_ADDR` | The instance's origin as clients reach it, such as `https://voice.example.org`: logins are signed for it and invitation links carry it |
| `DRYWIRE_PUBLIC_RELAY` | `DRYWIRE_UDP_ADDR` | The relay's address as clients reach it, such as `voice.example.org:10050` |
| `DRYWIRE_WEB_ADDR` | none (`make dev`: `127.0.0.1:10058`) | UDP address of the relay's WebTransport side, for browsers (ADR 0038). Unset, no browser can join |
| `DRYWIRE_PUBLIC_WEB_RELAY` | `DRYWIRE_WEB_ADDR` | That address as browsers reach it, `host:port` |
| `DRYWIRE_DB` | `drywire.db` (`make dev`: `.dev/drywire.db`) | SQLite database |
| `DRYWIRE_LEDGER` | `DRYWIRE_DB` + `.deletions` | Deletion ledger (ADR 0019); keep it out of backups |
| `DRYWIRE_SPACE_CREATORS` | nobody (`make dev`: `*`) | Who may create spaces: `*` for anyone, or public keys in hex, comma-separated |
| `DRYWIRE_MAX_MEMBERS` | `50` | Member limit of a new space |
| `DRYWIRE_TLS_CERT`, `DRYWIRE_TLS_KEY` | none | Serve HTTPS (TLS 1.3) from these PEM files |
| `DRYWIRE_TRUST_PROXY` | off | Where the reverse proxy connects from: `1` for this host's loopback, or private networks such as `172.18.0.0/16` for a proxy in a Docker network. Its requests name their client in `X-Forwarded-For` |
| `DRYWIRE_ACCESS_LOG` | `-` (standard output; the Docker image: `/data/access/`) | Access log: a directory ending in `/` for one file per UTC day, deleted after `DRYWIRE_ACCESS_LOG_DAYS`; a single file someone else rotates; or `off` |
| `DRYWIRE_ACCESS_LOG_DAYS` | `365` | How many days a directory access log keeps, at most 365 |

If an address is taken, the server stops with an error; it never picks
another port. Tokens cross the control plane's connections, so it serves
TLS itself or sits behind a reverse proxy that does: in plain HTTP it
listens on a loopback address only, unless `DRYWIRE_TRUST_PROXY` names
the private network of the proxy, and then it accepts connections from
that network and loopback alone (ADR 0027).

### Maintenance commands

The release image has nothing but the binary, so the binary does its own
maintenance (ADR 0025). Each command reads the same variables as the
server:

```sh
drywire-server health              # exit 0 if the control plane answers
drywire-server backup > copy.db    # consistent copy, while the server runs
drywire-server restore < copy.db   # server stopped: replace, then replay deletions
drywire-server creator-code        # a code that creates one space (ADR 0054)
```

`restore` checks the copy and replays the deletion ledger on it before it
touches anything, then puts it in place and says how many deletions it
applied again. Temporary copies that a killed `backup` or `restore` leaves
behind are removed at the next start. The
deployment ([`deploy/`](../deploy/README.md)) wraps them in `make backup`
and `make restore`, with encryption.

### Logs

Application logs (standard error) never hold an address, a token, a key or
anything a user wrote. The access log (JSON lines) is minimised as the spec
says (section 9): client address, method and route pattern
(`GET /v1/spaces/{space}`, never the path itself), status and duration; no
query string, header or `Referer`. Keep it a year at most.

### Browsers

Browsers cannot send UDP: with `DRYWIRE_WEB_ADDR` set, the server also
speaks WebTransport (HTTP/3) on that UDP port, at `/v1/relay`, and
browsers send the same packets as its datagrams (ADR 0038). The relay
copies packets between browsers and native clients alike. Its
certificate is made in memory, renewed every six days, and pinned by
the browsers with the hashes the join answer gives (ADR 0039). This
side uses `quic-go` and `webtransport-go` (MIT), the server's only
dependencies besides SQLite.

### A first space

With the server running, from `core/`:

```sh
cargo run -p drywire-cli -- create-space --instance http://127.0.0.1:10050 --name Team
cargo run -p drywire-cli -- invite      # prints an https://drywire.gg/i#… link
cargo run -p drywire-cli -- join        # talk
```

Someone else joins with `drywire join --link '<link>'`. On a real
instance, only the identities listed in `DRYWIRE_SPACE_CREATORS` create
spaces: `drywire identity` prints a device's key.

## Code

| Package | What it does |
| --- | --- |
| `cmd/drywire-server` | Reads the environment, opens the sockets, runs the relay and the HTTP API |
| `internal/relay` | The data plane: binding sessions, forwarding audio and announcements, rate limits, loss counting |
| `internal/wire` | Encodes and decodes packets as the relay sees them: header, control packets, replay window |
| `internal/store` | The control plane's lasting state in SQLite: identities, spaces, rooms, members, bans, invitation codes; schema migrations |
| `internal/ledger` | The deletion ledger, replayed at every start so a restored backup cannot bring deleted data back (ADR 0019) |
| `internal/auth` | Logs identities in: signed challenges, short-lived tokens in memory (ADR 0020) |
| `internal/presence` | Who is in which room right now, in memory only, and the subscriptions behind the event streams (ADR 0021) |
| `internal/ratelimit` | The token bucket behind every rate limit |
| `internal/api` | HTTP routes: health, version, status, and the control plane (login, spaces, rooms, members, bans, invitations, sessions, presence events), with per-address rate limits and the access log |
| `internal/clientref` | Reference of what clients do above the relay (key derivation, audio encryption, payload framing, signed announcements, invitation links); used only to build and check test vectors |
| `internal/vectors` | Builds, writes and reads the shared test vectors |
| `cmd/drywire-vectors` | Writes the vectors to `core/drywire-proto/vectors/` (`make vectors`) |
| `cmd/drywire-static` | Serves the web client's build in a deployment, beside the API, never inside it (ADR 0049) |

Start reading at `internal/relay/relay.go`: `Handle` follows protocol
section 10.1 step by step. Everything follows
[`docs/protocol.md`](../docs/protocol.md), and comments refer to its section
numbers.

## Test it

```sh
go test -race ./...   # or `make test` from the repository root
```

The relay tests drive `Handle` with a fake clock for exact rate-limit and
timeout checks, and run a real relay on a loopback socket with fake clients
that send the shared test vectors.
