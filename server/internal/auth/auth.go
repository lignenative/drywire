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
	"net/url"
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
// becomes "https://voice.example.org".
func CanonicalOrigin(address string) (string, error) {
	u, err := url.Parse(address)
	if err != nil {
		return "", err
	}
	scheme := strings.ToLower(u.Scheme)
	bad := func(why string) (string, error) {
		return "", fmt.Errorf("instance address %q: %s", address, why)
	}
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
	port := u.Port()
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
	var raw [32]byte
	rand.Read(raw[:])
	t := hex.EncodeToString(raw[:])
	until := now.Add(TokenLifetime)
	a.tokens[sha256.Sum256([]byte(t))] = token{key: key, expires: until}
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
