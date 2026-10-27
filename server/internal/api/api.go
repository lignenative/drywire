// Package api is the server's HTTP interface: what is running (health,
// version, status) and the control plane (docs/protocol.md, section 13):
// login, spaces, rooms, members, invitations, and the UDP sessions that let
// a member into a room.
//
// Handlers check who may do what, then call the store. Responses never
// contain an address, and never a secret other than the one the caller is
// being handed (its token, its session secret, a new invitation code).
package api

import (
	"encoding/json"
	"errors"
	"log/slog"
	"net"
	"net/http"
	"net/netip"
	"strings"
	"sync"
	"time"

	"github.com/lignenative/drywire/server/internal/auth"
	"github.com/lignenative/drywire/server/internal/ratelimit"
	"github.com/lignenative/drywire/server/internal/relay"
	"github.com/lignenative/drywire/server/internal/store"
	"github.com/lignenative/drywire/server/internal/wire"
)

// Limits of the control plane.
const (
	// maxBody is the largest request body read.
	maxBody = 64 << 10
	// Requests per second per client IP address, and the burst allowed.
	ipRate, ipBurst = 20, 60
	// maxTrackedIPs bounds the limiter's memory, as in the relay.
	maxTrackedIPs = 4096

	// Invitation defaults and bounds.
	defaultInviteLifetime = 7 * 24 * time.Hour
	maxInviteLifetime     = 30 * 24 * time.Hour
	defaultInviteUses     = 10
	maxInviteUses         = 100
)

// Config is what the operator decides.
type Config struct {
	// ServerVersion is reported by /v1/version.
	ServerVersion string
	// RelayAddr is the relay's UDP address as clients reach it
	// ("voice.example.org:10050"). It is handed out with each session.
	RelayAddr string
	// SpaceCreators may create spaces. Nil with AnyoneCreates false means
	// nobody can: the instance only serves the spaces it has.
	SpaceCreators []store.Key
	// AnyoneCreates lets every identity create spaces.
	AnyoneCreates bool
	// MaxMembers is the member limit of a new space.
	MaxMembers int
	// TrustProxy takes the client's address from the last
	// X-Forwarded-For entry, for a server behind a reverse proxy on this
	// host. Off, the peer's address is used.
	TrustProxy bool
}

// Server is the HTTP API. Create it with New.
type Server struct {
	cfg     Config
	store   *store.Store
	auth    *auth.Auth
	relay   *relay.Relay
	log     *slog.Logger // application log: no address, token or content
	access  *slog.Logger // access log: route, status, client address
	now     func() time.Time
	started time.Time

	limitMu  sync.Mutex
	ips      map[netip.Addr]*ratelimit.Bucket
	overflow ratelimit.Bucket
	swept    time.Time
}

// New returns the API. access may be nil to keep no access log.
func New(cfg Config, st *store.Store, a *auth.Auth, r *relay.Relay, log, access *slog.Logger, now func() time.Time) *Server {
	return &Server{
		cfg:      cfg,
		store:    st,
		auth:     a,
		relay:    r,
		log:      log,
		access:   access,
		now:      now,
		started:  now(),
		ips:      map[netip.Addr]*ratelimit.Bucket{},
		overflow: ratelimit.New(ipRate, ipBurst, now()),
	}
}

// Version is what GET /v1/version returns. Clients compare
// ProtocolVersions with their own to tell the user when to update
// (protocol section 11).
type Version struct {
	Server string `json:"server"`
	// []int, not []uint8: encoding/json writes a byte slice as base64.
	ProtocolVersions []int `json:"protocol_versions"`
}

// Status is what GET /v1/status returns.
type Status struct {
	Version
	UptimeSeconds int64 `json:"uptime_seconds"`
	relay.Stats
}

func (s *Server) version() Version {
	v := Version{Server: s.cfg.ServerVersion}
	for _, p := range wire.SupportedVersions {
		v.ProtocolVersions = append(v.ProtocolVersions, int(p))
	}
	return v
}

// Handler returns every route. Each pattern documents itself; the
// protocol's section 13 describes them.
func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/plain; charset=utf-8")
		w.Write([]byte("ok\n"))
	})
	mux.HandleFunc("GET /v1/version", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, http.StatusOK, s.version())
	})
	mux.HandleFunc("GET /v1/status", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, http.StatusOK, Status{
			Version:       s.version(),
			UptimeSeconds: int64(s.now().Sub(s.started).Seconds()),
			Stats:         s.relay.Stats(),
		})
	})

	mux.HandleFunc("POST /v1/auth/challenge", s.challenge)
	mux.HandleFunc("POST /v1/auth/login", s.login)

	mux.HandleFunc("GET /v1/spaces", s.authed(s.listSpaces))
	mux.HandleFunc("POST /v1/spaces", s.authed(s.createSpace))
	mux.HandleFunc("GET /v1/spaces/{space}", s.authed(s.getSpace))
	mux.HandleFunc("DELETE /v1/spaces/{space}", s.authed(s.deleteSpace))
	mux.HandleFunc("POST /v1/spaces/{space}/rooms", s.authed(s.createRoom))
	mux.HandleFunc("DELETE /v1/rooms/{room}", s.authed(s.deleteRoom))
	mux.HandleFunc("PATCH /v1/spaces/{space}/members/{key}", s.authed(s.setRole))
	mux.HandleFunc("DELETE /v1/spaces/{space}/members/{key}", s.authed(s.removeMember))
	mux.HandleFunc("GET /v1/spaces/{space}/bans", s.authed(s.listBans))
	mux.HandleFunc("POST /v1/spaces/{space}/bans", s.authed(s.ban))
	mux.HandleFunc("DELETE /v1/spaces/{space}/bans/{key}", s.authed(s.unban))
	mux.HandleFunc("GET /v1/spaces/{space}/invitations", s.authed(s.listInvitations))
	mux.HandleFunc("POST /v1/spaces/{space}/invitations", s.authed(s.createInvitation))
	mux.HandleFunc("DELETE /v1/spaces/{space}/invitations/{id}", s.authed(s.revokeInvitation))
	mux.HandleFunc("POST /v1/invitations/redeem", s.authed(s.redeem))

	return s.logged(s.limited(mux))
}

// caller is who sent an authenticated request. Identity is nil for a key
// that logged in but has no identity yet (it has not created a space or
// redeemed an invitation).
type caller struct {
	key      store.Key
	identity *store.Identity
}

// registered returns the caller's identity, or answers 403 if it has none.
func (c caller) registered(w http.ResponseWriter) (store.Identity, bool) {
	if c.identity == nil {
		writeError(w, http.StatusForbidden, "join a space with an invitation first")
		return store.Identity{}, false
	}
	return *c.identity, true
}

// authed checks the bearer token and passes the caller to h.
func (s *Server) authed(h func(http.ResponseWriter, *http.Request, caller)) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		token, ok := strings.CutPrefix(r.Header.Get("Authorization"), "Bearer ")
		key, valid := s.auth.Check(token)
		if !ok || !valid {
			w.Header().Set("WWW-Authenticate", "Bearer")
			writeError(w, http.StatusUnauthorized, "log in again")
			return
		}
		c := caller{key: key}
		id, err := s.store.IdentityByKey(r.Context(), key)
		switch {
		case err == nil:
			c.identity = &id
		case !errors.Is(err, store.ErrNotFound):
			s.fail(w, err)
			return
		}
		h(w, r, c)
	}
}

// role checks that the caller is a member of the space with at least the
// given role, and returns its role. A space the caller is not in answers
// 404, as if it did not exist.
func (s *Server) role(w http.ResponseWriter, r *http.Request, c caller, space store.ID, least store.Role) (store.Role, bool) {
	if c.identity == nil {
		writeError(w, http.StatusNotFound, "no such space")
		return "", false
	}
	role, err := s.store.Role(r.Context(), space, c.identity.ID)
	if errors.Is(err, store.ErrNotFound) {
		writeError(w, http.StatusNotFound, "no such space")
		return "", false
	}
	if err != nil {
		s.fail(w, err)
		return "", false
	}
	if role.Rank() < least.Rank() {
		writeError(w, http.StatusForbidden, "this needs the "+string(least)+" role")
		return "", false
	}
	return role, true
}

// pathID and pathKey read a hex value from the URL path; a malformed one
// answers 404 like an unknown one.
func pathID(w http.ResponseWriter, r *http.Request, name string) (store.ID, bool) {
	id, err := store.ParseID(r.PathValue(name))
	if err != nil {
		writeError(w, http.StatusNotFound, "not found")
	}
	return id, err == nil
}

func pathKey(w http.ResponseWriter, r *http.Request, name string) (store.Key, bool) {
	k, err := store.ParseKey(r.PathValue(name))
	if err != nil {
		writeError(w, http.StatusNotFound, "not found")
	}
	return k, err == nil
}

// readJSON decodes a request body of at most maxBody bytes. Unknown fields
// are ignored, so a newer client can talk to an older server.
func readJSON(w http.ResponseWriter, r *http.Request, v any) bool {
	err := json.NewDecoder(http.MaxBytesReader(w, r.Body, maxBody)).Decode(v)
	if err != nil {
		writeError(w, http.StatusBadRequest, "the request body is not the expected JSON")
		return false
	}
	return true
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(status)
	json.NewEncoder(w).Encode(v)
}

func writeError(w http.ResponseWriter, status int, message string) {
	writeJSON(w, status, map[string]string{"error": message})
}

// fail answers a store error: the known ones with their status and
// message, anything else with a 500 whose cause goes to the log only.
func (s *Server) fail(w http.ResponseWriter, err error) {
	switch {
	case errors.Is(err, store.ErrNotFound):
		writeError(w, http.StatusNotFound, "not found")
	case errors.Is(err, store.ErrBadName):
		writeError(w, http.StatusBadRequest, err.Error())
	case errors.Is(err, store.ErrNameTaken), errors.Is(err, store.ErrSpaceFull):
		writeError(w, http.StatusConflict, err.Error())
	case errors.Is(err, store.ErrBanned):
		writeError(w, http.StatusForbidden, err.Error())
	case errors.Is(err, store.ErrInvalidCode):
		writeError(w, http.StatusNotFound, err.Error())
	default:
		s.log.Error("request failed", "error", err)
		writeError(w, http.StatusInternalServerError, "internal error")
	}
}

// clientIP is the address the request came from: the peer, or behind a
// trusted reverse proxy the last X-Forwarded-For entry, which that proxy
// wrote (earlier entries come from the client and prove nothing).
func (s *Server) clientIP(r *http.Request) netip.Addr {
	if s.cfg.TrustProxy {
		if fwd := r.Header.Values("X-Forwarded-For"); len(fwd) > 0 {
			hops := strings.Split(fwd[len(fwd)-1], ",")
			if ip, err := netip.ParseAddr(strings.TrimSpace(hops[len(hops)-1])); err == nil {
				return ip.Unmap()
			}
		}
	}
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		return netip.Addr{}
	}
	ip, _ := netip.ParseAddr(host)
	return ip.Unmap()
}

// limited refuses requests beyond ipRate per client address.
func (s *Server) limited(h http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if !s.allow(s.clientIP(r)) {
			w.Header().Set("Retry-After", "1")
			writeError(w, http.StatusTooManyRequests, "too many requests, slow down")
			return
		}
		h.ServeHTTP(w, r)
	})
}

func (s *Server) allow(ip netip.Addr) bool {
	s.limitMu.Lock()
	defer s.limitMu.Unlock()
	now := s.now()
	if now.Sub(s.swept) >= time.Second {
		s.swept = now
		for a, b := range s.ips {
			if b.Full(now) {
				delete(s.ips, a)
			}
		}
	}
	b, ok := s.ips[ip]
	if !ok {
		if len(s.ips) >= maxTrackedIPs {
			return s.overflow.Allow(now, 1)
		}
		fresh := ratelimit.New(ipRate, ipBurst, now)
		b = &fresh
		s.ips[ip] = b
	}
	return b.Allow(now, 1)
}

// statusWriter remembers the status a handler wrote, for the access log.
type statusWriter struct {
	http.ResponseWriter
	status int
}

func (w *statusWriter) WriteHeader(status int) {
	if w.status == 0 {
		w.status = status
	}
	w.ResponseWriter.WriteHeader(status)
}

func (w *statusWriter) Write(b []byte) (int, error) {
	if w.status == 0 {
		w.status = http.StatusOK
	}
	return w.ResponseWriter.Write(b)
}

// Flush lets Server-Sent Events through the wrapper.
func (w *statusWriter) Flush() {
	if f, ok := w.ResponseWriter.(http.Flusher); ok {
		f.Flush()
	}
}

// logged writes one access log line per request, minimised as the spec
// says (section 9): the client address, the method, the route pattern
// instead of the path (so no id, key or code from a URL is ever written),
// the status and the duration. Never the query, the headers or the
// Referer.
func (s *Server) logged(h http.Handler) http.Handler {
	if s.access == nil {
		return h
	}
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		start := s.now()
		sw := &statusWriter{ResponseWriter: w}
		h.ServeHTTP(sw, r)
		route := r.Pattern
		if route == "" {
			route = r.Method + " (no route)"
		}
		s.access.Info("request",
			"ip", s.clientIP(r).String(),
			"route", route,
			"status", sw.status,
			"ms", s.now().Sub(start).Milliseconds())
	})
}
