package api

import (
	"bytes"
	"crypto/ed25519"
	"encoding/hex"
	"encoding/json"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/lignenative/drywire/server/internal/auth"
	"github.com/lignenative/drywire/server/internal/ledger"
	"github.com/lignenative/drywire/server/internal/relay"
	"github.com/lignenative/drywire/server/internal/store"
)

// These tests run the whole control plane, store and relay included, on a
// test HTTP server, with clients that log in like real ones.

type fakeClock struct {
	mu sync.Mutex
	t  time.Time
}

func (c *fakeClock) now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.t
}

func (c *fakeClock) advance(d time.Duration) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.t = c.t.Add(d)
}

// syncBuffer collects the access log.
type syncBuffer struct {
	mu sync.Mutex
	b  bytes.Buffer
}

func (b *syncBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.b.Write(p)
}

func (b *syncBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.b.String()
}

type harness struct {
	t      *testing.T
	url    string
	clock  *fakeClock
	relay  *relay.Relay
	store  *store.Store
	server *Server
	access *syncBuffer
	// ledgerPath is the deletion ledger's file.
	ledgerPath string
}

const instance = "https://voice.example.org"

func newHarness(t *testing.T, cfg Config) *harness {
	t.Helper()
	dir := t.TempDir()
	led, err := ledger.Open(filepath.Join(dir, "deletions"))
	if err != nil {
		t.Fatal(err)
	}
	clock := &fakeClock{t: time.Unix(1_800_000_000, 0)}
	st, err := store.Open(filepath.Join(dir, "db"), led, clock.now)
	if err != nil {
		t.Fatal(err)
	}
	quiet := slog.New(slog.NewTextHandler(io.Discard, nil))
	r := relay.New(clock.now, quiet)
	access := &syncBuffer{}
	if cfg.MaxMembers == 0 {
		cfg.MaxMembers = 50
	}
	if cfg.RelayAddr == "" {
		cfg.RelayAddr = "voice.example.org:10050"
	}
	cfg.ServerVersion = "test"
	s := New(cfg, st, auth.New(instance, clock.now), r, quiet, slog.New(slog.NewJSONHandler(access, nil)), clock.now)
	srv := httptest.NewServer(s.Handler())
	t.Cleanup(func() {
		srv.Close()
		st.Close()
		led.Close()
	})
	return &harness{t: t, url: srv.URL, clock: clock, relay: r, store: st, server: s, access: access,
		ledgerPath: led.Path()}
}

// client is one device with its identity key.
type client struct {
	h       *harness
	private ed25519.PrivateKey
	key     store.Key
	token   string
}

func (h *harness) client(seed byte) *client {
	private := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{seed}, ed25519.SeedSize))
	c := &client{h: h, private: private}
	copy(c.key[:], private.Public().(ed25519.PublicKey))
	return c
}

// do sends a request and decodes a JSON answer into out (if not nil),
// returning the status.
func (c *client) do(method, path string, body, out any) int {
	c.h.t.Helper()
	var reader io.Reader
	if body != nil {
		raw, _ := json.Marshal(body)
		reader = bytes.NewReader(raw)
	}
	req, _ := http.NewRequest(method, c.h.url+path, reader)
	if c.token != "" {
		req.Header.Set("Authorization", "Bearer "+c.token)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		c.h.t.Fatal(err)
	}
	defer resp.Body.Close()
	if out != nil && resp.StatusCode < 300 {
		if err := json.NewDecoder(resp.Body).Decode(out); err != nil {
			c.h.t.Fatalf("%s %s: %v", method, path, err)
		}
	}
	return resp.StatusCode
}

// must is do, failing the test unless the status is want.
func (c *client) must(want int, method, path string, body, out any) {
	c.h.t.Helper()
	if got := c.do(method, path, body, out); got != want {
		c.h.t.Fatalf("%s %s: status %d, want %d", method, path, got, want)
	}
}

func (c *client) login() *client {
	c.h.t.Helper()
	var ch struct {
		Challenge string `json:"challenge"`
	}
	c.must(200, "POST", "/v1/auth/challenge", nil, &ch)
	var challenge [auth.ChallengeSize]byte
	raw, _ := hex.DecodeString(ch.Challenge)
	copy(challenge[:], raw)
	sig := ed25519.Sign(c.private, auth.LoginMessage(challenge, instance))
	var tok struct {
		Token string `json:"token"`
	}
	c.must(200, "POST", "/v1/auth/login", map[string]string{
		"public_key": c.key.String(), "challenge": ch.Challenge, "signature": hex.EncodeToString(sig),
	}, &tok)
	c.token = tok.Token
	return c
}

func (c *client) createSpace(name string) SpaceView {
	c.h.t.Helper()
	var v SpaceView
	c.must(201, "POST", "/v1/spaces", map[string]string{"name": name, "display_name": "owner"}, &v)
	return v
}

// invite creates an invitation and returns its code.
func (c *client) invite(space store.ID, uses int) string {
	c.h.t.Helper()
	var inv struct {
		Code     string `json:"code"`
		Instance string `json:"instance"`
	}
	c.must(201, "POST", "/v1/spaces/"+space.String()+"/invitations", map[string]int{"max_uses": uses}, &inv)
	if inv.Instance != instance {
		c.h.t.Fatalf("instance %q", inv.Instance)
	}
	return inv.Code
}

func (c *client) redeem(code, name string) int {
	c.h.t.Helper()
	return c.do("POST", "/v1/invitations/redeem", map[string]string{"code": code, "display_name": name}, nil)
}

func TestRoutes(t *testing.T) {
	h := newHarness(t, Config{})
	c := h.client(1)

	if status := c.do("GET", "/healthz", nil, nil); status != http.StatusOK {
		t.Fatalf("/healthz: status %d", status)
	}
	// Compare the raw body: clients in other languages read this JSON.
	resp, err := http.Get(h.url + "/v1/version")
	if err != nil {
		t.Fatal(err)
	}
	body, _ := io.ReadAll(resp.Body)
	resp.Body.Close()
	if got, want := string(body), `{"server":"test","protocol_versions":[1]}`+"\n"; got != want {
		t.Fatalf("/v1/version = %s, want %s", got, want)
	}

	var s map[string]any
	c.must(200, "GET", "/v1/status", nil, &s)
	for _, key := range []string{"server", "protocol_versions", "uptime_seconds", "rooms", "bound_sessions", "packets_dropped"} {
		if _, ok := s[key]; !ok {
			t.Errorf("/v1/status has no %q field: %v", key, s)
		}
	}
	if status := c.do("GET", "/nothing-here", nil, nil); status != http.StatusNotFound {
		t.Fatalf("unknown path: status %d, want 404", status)
	}
}

func TestEveryControlRouteNeedsAToken(t *testing.T) {
	h := newHarness(t, Config{AnyoneCreates: true})
	anon := h.client(1)
	if status := anon.do("GET", "/v1/spaces", nil, nil); status != http.StatusUnauthorized {
		t.Fatalf("no token: %d", status)
	}
	anon.token = strings.Repeat("ab", 32)
	if status := anon.do("POST", "/v1/spaces", map[string]string{"name": "x"}, nil); status != http.StatusUnauthorized {
		t.Fatalf("made-up token: %d", status)
	}
	c := h.client(1).login()
	h.clock.advance(auth.TokenLifetime)
	if status := c.do("GET", "/v1/spaces", nil, nil); status != http.StatusUnauthorized {
		t.Fatalf("expired token: %d", status)
	}
}

func TestOnlyListedIdentitiesCreateSpaces(t *testing.T) {
	h := newHarness(t, Config{})
	alex, sam := h.client(1), h.client(2)
	h.server.cfg.SpaceCreators = []store.Key{alex.key}
	alex.login()
	sam.login()
	if status := sam.do("POST", "/v1/spaces", map[string]string{"name": "Team", "display_name": "sam"}, nil); status != http.StatusForbidden {
		t.Fatalf("sam created a space: %d", status)
	}
	v := alex.createSpace("Team")
	if v.Role != store.RoleOwner || len(v.Rooms) != 1 || v.Rooms[0].Name != "general" || len(v.Members) != 1 {
		t.Fatalf("new space %+v", v)
	}
}

func TestAnInvitationLetsSomeoneIn(t *testing.T) {
	h := newHarness(t, Config{AnyoneCreates: true})
	alex, sam := h.client(1).login(), h.client(2).login()
	v := alex.createSpace("Team")
	space := "/v1/spaces/" + v.Space.ID.String()

	// Before the invitation, the space does not exist for sam.
	if status := sam.do("GET", space, nil, nil); status != http.StatusNotFound {
		t.Fatalf("outsider sees the space: %d", status)
	}
	code := alex.invite(v.Space.ID, 1)
	if status := sam.redeem(strings.Repeat("00", 16), "sam"); status != http.StatusNotFound {
		t.Fatalf("wrong code: %d", status)
	}
	if status := sam.redeem(code, "sam"); status != http.StatusOK {
		t.Fatalf("redeem: %d", status)
	}
	var got SpaceView
	sam.must(200, "GET", space, nil, &got)
	if got.Role != store.RoleMember || len(got.Members) != 2 || got.Members[1].Name != "sam" {
		t.Fatalf("sam's view %+v", got)
	}
	// The code was for one use.
	if status := h.client(3).login().redeem(code, "kim"); status != http.StatusNotFound {
		t.Fatalf("second use: %d", status)
	}
	// A member cannot invite, nor list invitations.
	if status := sam.do("POST", space+"/invitations", map[string]int{}, nil); status != http.StatusForbidden {
		t.Fatalf("member invites: %d", status)
	}
	var invs []store.Invitation
	alex.must(200, "GET", space+"/invitations", nil, &invs)
	if len(invs) != 0 {
		t.Fatalf("used-up invitation still listed: %+v", invs)
	}
	if status := alex.do("POST", space+"/invitations", map[string]int{"max_uses": 1000}, nil); status != http.StatusBadRequest {
		t.Fatalf("1000 uses: %d", status)
	}
}

func TestRolesDecideWhoActsOnWhom(t *testing.T) {
	h := newHarness(t, Config{AnyoneCreates: true})
	alex := h.client(1).login()
	v := alex.createSpace("Team")
	space := "/v1/spaces/" + v.Space.ID.String()
	sam, kim, lee := h.client(2).login(), h.client(3).login(), h.client(4).login()
	for _, c := range []*client{sam, kim, lee} {
		if status := c.redeem(alex.invite(v.Space.ID, 1), "someone"); status != http.StatusOK {
			t.Fatal(status)
		}
	}
	member := func(c *client) string { return space + "/members/" + c.key.String() }

	// Only the owner changes roles.
	if status := sam.do("PATCH", member(kim), map[string]string{"role": "admin"}, nil); status != http.StatusForbidden {
		t.Fatalf("member promotes: %d", status)
	}
	alex.must(204, "PATCH", member(sam), map[string]string{"role": "admin"}, nil)

	// An admin manages rooms and removes members, but not other admins.
	var room store.Room
	sam.must(201, "POST", space+"/rooms", map[string]string{"name": "scrims"}, &room)
	if status := kim.do("DELETE", "/v1/rooms/"+room.ID.String(), nil, nil); status != http.StatusForbidden {
		t.Fatalf("member deletes a room: %d", status)
	}
	sam.must(204, "DELETE", member(kim), nil, nil)
	alex.must(204, "PATCH", member(lee), map[string]string{"role": "admin"}, nil)
	if status := sam.do("DELETE", member(lee), nil, nil); status != http.StatusForbidden {
		t.Fatalf("admin removes admin: %d", status)
	}
	if status := sam.do("DELETE", member(alex), nil, nil); status != http.StatusForbidden {
		t.Fatalf("admin removes owner: %d", status)
	}

	// The owner cannot leave, but can hand the space over and then leave.
	if status := alex.do("DELETE", member(alex), nil, nil); status != http.StatusConflict {
		t.Fatalf("owner leaves: %d", status)
	}
	alex.must(204, "PATCH", member(lee), map[string]string{"role": "owner"}, nil)
	alex.must(204, "DELETE", member(alex), nil, nil)
	var got SpaceView
	lee.must(200, "GET", space, nil, &got)
	if got.Role != store.RoleOwner || len(got.Members) != 2 {
		t.Fatalf("after the handover: %+v", got)
	}
	// Only the owner deletes the space.
	if status := sam.do("DELETE", space, nil, nil); status != http.StatusForbidden {
		t.Fatalf("admin deletes the space: %d", status)
	}
	lee.must(204, "DELETE", space, nil, nil)
	if status := sam.do("GET", space, nil, nil); status != http.StatusNotFound {
		t.Fatalf("deleted space: %d", status)
	}
}

func TestABannedKeyCannotComeBack(t *testing.T) {
	h := newHarness(t, Config{AnyoneCreates: true})
	alex, sam := h.client(1).login(), h.client(2).login()
	v := alex.createSpace("Team")
	space := "/v1/spaces/" + v.Space.ID.String()
	sam.redeem(alex.invite(v.Space.ID, 1), "sam")

	if status := sam.do("POST", space+"/bans", map[string]string{"public_key": alex.key.String()}, nil); status != http.StatusForbidden {
		t.Fatalf("member bans: %d", status)
	}
	alex.must(204, "POST", space+"/bans", map[string]string{"public_key": sam.key.String()}, nil)
	if status := sam.do("GET", space, nil, nil); status != http.StatusNotFound {
		t.Fatalf("banned member still sees the space: %d", status)
	}
	if status := sam.redeem(alex.invite(v.Space.ID, 1), "sam"); status != http.StatusForbidden {
		t.Fatalf("banned key redeems: %d", status)
	}
	var bans []store.Key
	alex.must(200, "GET", space+"/bans", nil, &bans)
	if len(bans) != 1 || bans[0] != sam.key {
		t.Fatalf("bans %v", bans)
	}
	alex.must(204, "DELETE", space+"/bans/"+sam.key.String(), nil, nil)
	if status := sam.redeem(alex.invite(v.Space.ID, 1), "sam"); status != http.StatusOK {
		t.Fatalf("after unban: %d", status)
	}
}

func TestTheAccessLogHoldsRoutesNotPaths(t *testing.T) {
	h := newHarness(t, Config{AnyoneCreates: true})
	alex := h.client(1).login()
	v := alex.createSpace("Team")
	alex.must(200, "GET", "/v1/spaces/"+v.Space.ID.String()+"?secret=1", nil, nil)

	log := h.access.String()
	if !strings.Contains(log, `"route":"GET /v1/spaces/{space}"`) || !strings.Contains(log, `"ip":"127.0.0.1"`) {
		t.Fatalf("access log:\n%s", log)
	}
	for _, leak := range []string{v.Space.ID.String(), "secret", alex.token, alex.key.String()} {
		if strings.Contains(log, leak) {
			t.Fatalf("the access log holds %q:\n%s", leak, log)
		}
	}
}

func TestRequestsArePacedPerAddress(t *testing.T) {
	h := newHarness(t, Config{})
	c := h.client(1)
	for range ipBurst {
		c.must(200, "GET", "/v1/version", nil, nil)
	}
	if status := c.do("GET", "/v1/version", nil, nil); status != http.StatusTooManyRequests {
		t.Fatalf("over the burst: %d", status)
	}
	h.clock.advance(time.Second)
	c.must(200, "GET", "/v1/version", nil, nil)
}

func TestTheClientAddressComesFromATrustedProxyOnly(t *testing.T) {
	s := &Server{}
	r := httptest.NewRequest("GET", "/", nil)
	r.RemoteAddr = "127.0.0.1:5000"
	r.Header.Add("X-Forwarded-For", "203.0.113.9, 198.51.100.7")
	if got := s.clientIP(r).String(); got != "127.0.0.1" {
		t.Fatalf("untrusted: %s", got)
	}
	s.cfg.TrustProxy = true
	if got := s.clientIP(r).String(); got != "198.51.100.7" {
		t.Fatalf("trusted: %s (the last entry is the one the proxy wrote)", got)
	}
}
