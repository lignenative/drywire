package auth

import (
	"crypto/ed25519"
	"testing"
	"time"

	"github.com/lignenative/drywire/server/internal/store"
)

type fakeClock struct{ t time.Time }

func (c *fakeClock) now() time.Time          { return c.t }
func (c *fakeClock) advance(d time.Duration) { c.t = c.t.Add(d) }

const audience = "https://voice.example.org"

func identity() (ed25519.PrivateKey, store.Key) {
	private := ed25519.NewKeyFromSeed(make([]byte, ed25519.SeedSize))
	var key store.Key
	copy(key[:], private.Public().(ed25519.PublicKey))
	return private, key
}

func TestASignedChallengeGivesAToken(t *testing.T) {
	clock := &fakeClock{t: time.Unix(1_800_000_000, 0)}
	a := New(audience, clock.now)
	private, key := identity()

	c, err := a.Challenge()
	if err != nil {
		t.Fatal(err)
	}
	tok, until, err := a.Login(key, c, ed25519.Sign(private, LoginMessage(c, audience)))
	if err != nil {
		t.Fatal(err)
	}
	if until != clock.t.Add(TokenLifetime) {
		t.Fatalf("expires %v", until)
	}
	if got, ok := a.Check(tok); !ok || got != key {
		t.Fatal("the token is not valid")
	}
	// A challenge answers one login only.
	if _, _, err := a.Login(key, c, ed25519.Sign(private, LoginMessage(c, audience))); err != ErrDenied {
		t.Fatalf("challenge reused: %v", err)
	}
	clock.advance(TokenLifetime)
	if _, ok := a.Check(tok); ok {
		t.Fatal("an expired token is still valid")
	}
}

func TestLoginsThatMustFail(t *testing.T) {
	clock := &fakeClock{t: time.Unix(1_800_000_000, 0)}
	a := New(audience, clock.now)
	private, key := identity()
	other := store.Key{1}

	for name, try := range map[string]func(c [ChallengeSize]byte) error{
		"another instance's audience": func(c [ChallengeSize]byte) error {
			_, _, err := a.Login(key, c, ed25519.Sign(private, LoginMessage(c, "https://evil.example")))
			return err
		},
		"another key": func(c [ChallengeSize]byte) error {
			_, _, err := a.Login(other, c, ed25519.Sign(private, LoginMessage(c, audience)))
			return err
		},
		"a short signature": func(c [ChallengeSize]byte) error {
			_, _, err := a.Login(key, c, []byte{1, 2, 3})
			return err
		},
		"an expired challenge": func(c [ChallengeSize]byte) error {
			clock.advance(ChallengeLifetime)
			_, _, err := a.Login(key, c, ed25519.Sign(private, LoginMessage(c, audience)))
			return err
		},
		"a challenge the server never gave": func([ChallengeSize]byte) error {
			var c [ChallengeSize]byte
			_, _, err := a.Login(key, c, ed25519.Sign(private, LoginMessage(c, audience)))
			return err
		},
	} {
		c, err := a.Challenge()
		if err != nil {
			t.Fatal(err)
		}
		if err := try(c); err != ErrDenied {
			t.Errorf("%s: %v", name, err)
		}
	}
}

func TestUnansweredChallengesAreBounded(t *testing.T) {
	clock := &fakeClock{t: time.Unix(1_800_000_000, 0)}
	a := New(audience, clock.now)
	for range maxChallenges {
		if _, err := a.Challenge(); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := a.Challenge(); err != ErrBusy {
		t.Fatalf("challenge %d: %v", maxChallenges+1, err)
	}
	clock.advance(ChallengeLifetime)
	if _, err := a.Challenge(); err != nil {
		t.Fatalf("after expiry: %v", err)
	}
}

func TestRevokeEndsAnIdentitysTokens(t *testing.T) {
	a := New(audience, time.Now)
	private, key := identity()
	c, _ := a.Challenge()
	tok, _, err := a.Login(key, c, ed25519.Sign(private, LoginMessage(c, audience)))
	if err != nil {
		t.Fatal(err)
	}
	a.Revoke(key)
	if _, ok := a.Check(tok); ok {
		t.Fatal("a revoked token is still valid")
	}
}

func TestCanonicalOrigin(t *testing.T) {
	for in, want := range map[string]string{
		"https://Voice.Example.org":      "https://voice.example.org",
		"HTTPS://voice.example.org:443/": "https://voice.example.org",
		"https://voice.example.org:8443": "https://voice.example.org:8443",
		"http://127.0.0.1:10050":         "http://127.0.0.1:10050",
		"http://localhost:80":            "http://localhost",
		"http://[::1]:10050":             "http://[::1]:10050",
		"https://[2001:DB8::1]":          "https://[2001:db8::1]",
	} {
		if got, err := CanonicalOrigin(in); err != nil || got != want {
			t.Errorf("CanonicalOrigin(%q) = %q, %v; want %q", in, got, err, want)
		}
	}
	for _, in := range []string{"voice.example.org", "ftp://voice.example.org", "https://a@voice.example.org",
		"https://voice.example.org/path", "https://voice.example.org/?q", "https://voice.example.org#f", "https://"} {
		if got, err := CanonicalOrigin(in); err == nil {
			t.Errorf("CanonicalOrigin(%q) = %q, accepted", in, got)
		}
	}
}

func TestAnIdentityHoldsFewTokens(t *testing.T) {
	clock := &fakeClock{t: time.Unix(1_800_000_000, 0)}
	a := New(audience, clock.now)
	private, key := identity()
	var tokens []string
	for range tokensPerKey + 1 {
		clock.advance(time.Second)
		c, _ := a.Challenge()
		tok, _, err := a.Login(key, c, ed25519.Sign(private, LoginMessage(c, audience)))
		if err != nil {
			t.Fatal(err)
		}
		tokens = append(tokens, tok)
	}
	if _, ok := a.Check(tokens[0]); ok {
		t.Fatal("the oldest token survived one login too many")
	}
	for _, tok := range tokens[1:] {
		if _, ok := a.Check(tok); !ok {
			t.Fatal("a recent token was dropped")
		}
	}
}
