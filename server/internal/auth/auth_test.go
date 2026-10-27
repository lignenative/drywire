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
	for _, c := range OriginCases {
		got, err := CanonicalOrigin(c.Input)
		if c.Canonical == "" && err == nil {
			t.Errorf("CanonicalOrigin(%q) = %q, accepted", c.Input, got)
		}
		if c.Canonical != "" && (err != nil || got != c.Canonical) {
			t.Errorf("CanonicalOrigin(%q) = %q, %v; want %q", c.Input, got, err, c.Canonical)
		}
	}
}
