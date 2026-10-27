// Package auth logs identities in to the control plane: a signed
// challenge, then a short-lived bearer token (docs/protocol.md, section
// 13.2). There is no account and no password: a device proves it holds the
// private key of its identity.
//
// Challenges and tokens live in memory only. A restart logs everyone out,
// and clients log in again without asking their user anything; in return no
// token ever reaches the disk or a backup.
package auth

import (
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"net"
	"net/netip"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/lignenative/drywire/server/internal/store"
)

const (
	// ChallengeSize is the size of a login challenge.
	ChallengeSize = 32
	// ChallengeLifetime is how long a challenge can be answered.
	ChallengeLifetime = time.Minute
	// TokenLifetime is how long a session token is valid. Clients log in
	// again when it expires, which also keeps last_seen_at fresh.
	TokenLifetime = time.Hour

	// maxChallenges bounds the memory unanswered challenges can take.
	maxChallenges = 10_000
	// maxTokens bounds the valid tokens, and tokensPerKey those of one
	// identity: logging in again drops its oldest token. Anyone can log in
	// with a fresh key, so neither may grow without limit.
	maxTokens     = 100_000
	tokensPerKey  = 4
	sweepInterval = 10 * time.Second
)

// loginInfo starts every signed login message, so a login signature can
// never pass for an announcement or anything else the key signs.
const loginInfo = "drywire/login/v1"

var (
	// ErrBusy means too many challenges are waiting for an answer.
	ErrBusy = errors.New("too many logins in progress, try again shortly")
	// ErrDenied covers every failed login: unknown or expired challenge,
	// bad signature. The client is not told which.
	ErrDenied = errors.New("login refused")
)

// LoginMessage is what a client signs to log in (protocol section 13.2):
//
//	"drywire/login/v1" ‖ challenge (32) ‖ audience
//
// The audience is the instance's origin, as the client reached it. Signing
// it means a malicious instance cannot pass on another instance's challenge
// and log in there with the answer.
func LoginMessage(challenge [ChallengeSize]byte, audience string) []byte {
	msg := make([]byte, 0, len(loginInfo)+ChallengeSize+len(audience))
	msg = append(msg, loginInfo...)
	msg = append(msg, challenge[:]...)
	return append(msg, audience...)
}

// CanonicalOrigin turns an instance address into the form both sides sign
// and the invitation link holds: lowercase scheme and host, no default
// port, no path, query, fragment or user name. "https://Voice.Example.org:443/"
// becomes "https://voice.example.org". The host is a name of ASCII
// letters, digits, dots and hyphens, or an IP address; the port, if any,
// a number from 1 to 65535 without leading zeros. The Rust clients follow
// the same rules, checked by the shared vectors' "origins".
func CanonicalOrigin(address string) (string, error) {
	bad := func(why string) (string, error) {
		return "", fmt.Errorf("instance address %q: %s", address, why)
	}
	u, err := url.Parse(address)
	if err != nil {
		return bad("not a URL")
	}
	scheme := strings.ToLower(u.Scheme)
	if scheme != "https" && scheme != "http" {
		return bad("must start with https:// or http://")
	}
	if u.Opaque != "" || u.User != nil || (u.Path != "" && u.Path != "/") || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" {
		return bad("must be a scheme, a host and an optional port, nothing else")
	}
	host := strings.ToLower(u.Hostname())
	if host == "" {
		return bad("has no host")
	}
	if strings.HasPrefix(u.Host, "[") {
		if ip, err := netip.ParseAddr(host); err != nil || !ip.Is6() || ip.Zone() != "" {
			return bad("has a bad IPv6 address")
		}
	} else if strings.Trim(u.Hostname(), "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-") != "" {
		// Checked before lowercasing, which turns some non-ASCII
		// letters (the Kelvin sign) into ASCII ones.
		return bad("the host may only hold ASCII letters, digits, dots and hyphens")
	}
	port := u.Port()
	if strings.HasSuffix(u.Host, ":") {
		return bad("has a bad port")
	}
	if port != "" {
		n, err := strconv.Atoi(port)
		if err != nil || n < 1 || n > 65535 || port[0] == '0' {
			return bad("has a bad port")
		}
	}
	if (scheme == "https" && port == "443") || (scheme == "http" && port == "80") {
		port = ""
	}
	if port != "" {
		return scheme + "://" + net.JoinHostPort(host, port), nil
	}
	if strings.Contains(host, ":") {
		host = "[" + host + "]"
	}
	return scheme + "://" + host, nil
}

// Auth holds the challenges waiting for an answer and the valid tokens. It
// is safe for concurrent use.
type Auth struct {
	mu        sync.Mutex
	now       func() time.Time
	audience  string
	lastSweep time.Time

	challenges map[[ChallengeSize]byte]time.Time // expiry
	// Tokens are found by their SHA-256, so the map lookup never compares
	// a token byte by byte against a guess.
	tokens map[[sha256.Size]byte]token
}

type token struct {
	key     store.Key
	issued  time.Time
	expires time.Time
}

// New returns an Auth for the instance at audience (a canonical origin).
// now is the clock; pass time.Now outside tests.
func New(audience string, now func() time.Time) *Auth {
	return &Auth{
		now:        now,
		audience:   audience,
		challenges: map[[ChallengeSize]byte]time.Time{},
		tokens:     map[[sha256.Size]byte]token{},
	}
}

// Audience is the origin clients must sign.
func (a *Auth) Audience() string { return a.audience }

// Challenge returns a fresh challenge, valid for ChallengeLifetime and for
// one login attempt.
func (a *Auth) Challenge() ([ChallengeSize]byte, error) {
	var c [ChallengeSize]byte
	a.mu.Lock()
	defer a.mu.Unlock()
	now := a.now()
	a.sweep(now)
	if len(a.challenges) >= maxChallenges {
		return c, ErrBusy
	}
	rand.Read(c[:]) // never fails (crypto/rand documentation)
	a.challenges[c] = now.Add(ChallengeLifetime)
	return c, nil
}

// Login checks the signature of the login message with key and, if it is
// valid, returns a new token and when it expires. The challenge is used up
// either way.
func (a *Auth) Login(key store.Key, challenge [ChallengeSize]byte, signature []byte) (string, time.Time, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	now := a.now()
	expires, ok := a.challenges[challenge]
	delete(a.challenges, challenge)
	if !ok || !now.Before(expires) {
		return "", time.Time{}, ErrDenied
	}
	if len(signature) != ed25519.SignatureSize ||
		!ed25519.Verify(ed25519.PublicKey(key[:]), LoginMessage(challenge, a.audience), signature) {
		return "", time.Time{}, ErrDenied
	}
	a.sweep(now)
	a.dropOldest(key)
	if len(a.tokens) >= maxTokens {
		return "", time.Time{}, ErrBusy
	}
	var raw [32]byte
	rand.Read(raw[:])
	t := hex.EncodeToString(raw[:])
	until := now.Add(TokenLifetime)
	a.tokens[sha256.Sum256([]byte(t))] = token{key: key, issued: now, expires: until}
	return t, until, nil
}

// Check returns the identity a token was issued to, if it is still valid.
func (a *Auth) Check(t string) (store.Key, bool) {
	a.mu.Lock()
	defer a.mu.Unlock()
	tok, ok := a.tokens[sha256.Sum256([]byte(t))]
	if !ok || !a.now().Before(tok.expires) {
		return store.Key{}, false
	}
	return tok.key, true
}

// dropOldest makes room for one more token of key, ending its oldest
// tokens beyond tokensPerKey - 1.
func (a *Auth) dropOldest(key store.Key) {
	for {
		var oldest [sha256.Size]byte
		count := 0
		var at time.Time
		for h, tok := range a.tokens {
			if tok.key != key {
				continue
			}
			count++
			if at.IsZero() || tok.issued.Before(at) {
				oldest, at = h, tok.issued
			}
		}
		if count < tokensPerKey {
			return
		}
		delete(a.tokens, oldest)
	}
}

// Revoke ends every token of an identity, when it is deleted.
func (a *Auth) Revoke(key store.Key) {
	a.mu.Lock()
	defer a.mu.Unlock()
	for h, tok := range a.tokens {
		if tok.key == key {
			delete(a.tokens, h)
		}
	}
}

// sweep forgets expired challenges and tokens, at most every sweepInterval.
func (a *Auth) sweep(now time.Time) {
	if now.Sub(a.lastSweep) < sweepInterval {
		return
	}
	a.lastSweep = now
	for c, expires := range a.challenges {
		if !now.Before(expires) {
			delete(a.challenges, c)
		}
	}
	for h, tok := range a.tokens {
		if !now.Before(tok.expires) {
			delete(a.tokens, h)
		}
	}
}

// OriginCase is one instance address and its canonical form, empty if it
// must be refused.
type OriginCase struct {
	Input     string `json:"input"`
	Canonical string `json:"canonical"`
}

// OriginCases are the addresses both implementations are tested on: they
// go into the shared vectors, which the Rust clients read.
var OriginCases = []OriginCase{
	{"https://Voice.Example.org", "https://voice.example.org"},
	{"HTTPS://voice.example.org:443/", "https://voice.example.org"},
	{"https://voice.example.org:8443", "https://voice.example.org:8443"},
	{"http://127.0.0.1:10050", "http://127.0.0.1:10050"},
	{"http://localhost:80", "http://localhost"},
	{"http://[::1]:10050", "http://[::1]:10050"},
	{"https://[2001:DB8::1]", "https://[2001:db8::1]"},
	{"https://xn--bcher-kva.example", "https://xn--bcher-kva.example"},
	{"voice.example.org", ""},
	{"ftp://voice.example.org", ""},
	{"https://a@voice.example.org", ""},
	{"https://voice.example.org/path", ""},
	{"https://voice.example.org/?q", ""},
	{"https://voice.example.org#f", ""},
	{"https://", ""},
	{"https://host:", ""},
	{"https://host:0", ""},
	{"https://host:0443", ""},
	{"https://host:99999", ""},
	{"https://host:+1", ""},
	{"https://bücher.example", ""},
	{"https://\u212aelvin.example", ""},
	{"https://ho_st.example", ""},
	{"http://[fe80::1%25en0]:10050", ""},
	{"https://[not:an:ip:zz]", ""},
}
