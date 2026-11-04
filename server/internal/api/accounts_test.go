package api

import (
	"context"
	"crypto/ed25519"
	"encoding/hex"
	"net/http"
	"regexp"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/lignenative/drywire/server/internal/auth"
	"github.com/lignenative/drywire/server/internal/mail"
	"github.com/lignenative/drywire/server/internal/store"
)

// outbox keeps the messages the server sends.
type outbox struct {
	mu   sync.Mutex
	sent []mail.Message
}

func (o *outbox) Send(_ context.Context, m mail.Message) error {
	o.mu.Lock()
	defer o.mu.Unlock()
	o.sent = append(o.sent, m)
	return nil
}

// code reads the last code sent to an address.
func (o *outbox) code(t *testing.T, to string) string {
	t.Helper()
	o.mu.Lock()
	defer o.mu.Unlock()
	for i := len(o.sent) - 1; i >= 0; i-- {
		if o.sent[i].To == to {
			return regexp.MustCompile(`^\d{6}`).FindString(o.sent[i].Subject)
		}
	}
	t.Fatalf("no mail to %s", to)
	return ""
}

// signIn asks a code for the address and sends it back with the device's
// signed challenge, as the apps do. It returns the status and the answer.
func (c *client) signIn(o *outbox, email string, code func(string) string) (int, map[string]any) {
	c.h.t.Helper()
	c.h.clock.advance(time.Second)
	if status := c.do("POST", "/v1/auth/email", map[string]string{"email": email, "lang": "fr"}, nil); status != http.StatusAccepted {
		c.h.t.Fatalf("asking a code: %d", status)
	}
	c.h.clock.advance(time.Second)
	var ch struct {
		Challenge string `json:"challenge"`
	}
	c.must(200, "POST", "/v1/auth/challenge", nil, &ch)
	var challenge [auth.ChallengeSize]byte
	raw, _ := hex.DecodeString(ch.Challenge)
	copy(challenge[:], raw)
	sig := ed25519.Sign(c.private, auth.LoginMessage(challenge, instance))
	c.h.clock.advance(time.Second)
	var out map[string]any
	status := c.do("POST", "/v1/auth/email/verify", map[string]string{
		"email": email, "code": code(o.code(c.h.t, strings.ToLower(strings.TrimSpace(email)))),
		"public_key": c.key.String(), "challenge": ch.Challenge, "signature": hex.EncodeToString(sig),
	}, &out)
	if tok, ok := out["token"].(string); ok {
		c.token = tok
	}
	return status, out
}

func same(code string) string { return code }

func TestSigningUpIsAnAddressACodeAndAUserName(t *testing.T) {
	o := &outbox{}
	h := newHarness(t, Config{AnyoneCreates: true, Mail: o})
	pc := h.client(1)

	status, out := pc.signIn(o, " Alex@Example.com", func(c string) string { return c[:3] + " " + c[3:] })
	if status != http.StatusOK || out["new_account"] != true || out["username"] != "" {
		t.Fatalf("sign up: %d %v", status, out)
	}
	if subject := o.sent[0].Subject; !strings.HasSuffix(subject, " est votre code Drywire") {
		t.Fatalf("subject %q", subject)
	}
	var taken struct {
		Suggestions []string `json:"suggestions"`
	}
	pc.must(http.StatusOK, "PUT", "/v1/me/username", map[string]string{"username": "alex", "display_name": "Alex"}, nil)
	var me map[string]any
	pc.must(http.StatusOK, "GET", "/v1/me", nil, &me)
	if me["username"] != "alex" || me["email"] != "alex@example.com" || me["name"] != "Alex" {
		t.Fatalf("me %v", me)
	}

	// The same address on a phone: the same account, two devices.
	phone := h.client(2)
	status, out = phone.signIn(o, "alex@example.com", same)
	if status != http.StatusOK || out["new_account"] != false || out["username"] != "alex" {
		t.Fatalf("second device: %d %v", status, out)
	}
	var devices []map[string]any
	phone.must(http.StatusOK, "GET", "/v1/me/devices", nil, &devices)
	if len(devices) != 2 || devices[1]["this_device"] != true {
		t.Fatalf("devices %v", devices)
	}

	// Sam cannot have "alex", and gets free names instead.
	sam := h.client(3)
	sam.signIn(o, "sam@example.com", same)
	if status := sam.do("PUT", "/v1/me/username", map[string]string{"username": "Alex"}, &taken); status != http.StatusConflict || len(taken.Suggestions) != 2 {
		t.Fatalf("taken: %d %v", status, taken)
	}
	if status := sam.do("PUT", "/v1/me/username", map[string]string{"username": "a"}, nil); status != http.StatusBadRequest {
		t.Fatalf("too short: %d", status)
	}
	sam.must(http.StatusOK, "PUT", "/v1/me/username", map[string]string{"username": "sam"}, nil)

	// Alex signs the phone out from the PC: its token ends.
	pc.must(http.StatusNoContent, "DELETE", "/v1/me/devices/"+phone.key.String(), nil, nil)
	if status := phone.do("GET", "/v1/me", nil, nil); status != http.StatusUnauthorized {
		t.Fatalf("a removed device: %d", status)
	}
}

func TestAWrongCodeSpendsATry(t *testing.T) {
	o := &outbox{}
	h := newHarness(t, Config{Mail: o})
	c := h.client(1)
	status, out := c.signIn(o, "alex@example.com", func(string) string { return "000000" })
	if status != http.StatusUnauthorized || out["tries_left"] != float64(4) {
		t.Fatalf("wrong code: %d %v", status, out)
	}
	if _, err := h.store.IdentityByKey(context.Background(), c.key); err == nil {
		t.Fatal("a wrong code made an account")
	}
	// No mail on an instance that sends none.
	h2 := newHarness(t, Config{})
	if status := h2.client(1).do("POST", "/v1/auth/email", map[string]string{"email": "alex@example.com"}, nil); status != http.StatusServiceUnavailable {
		t.Fatalf("no mail: %d", status)
	}
}

func TestPeopleInviteEachOtherByUserName(t *testing.T) {
	o := &outbox{}
	h := newHarness(t, Config{AnyoneCreates: true, Mail: o})
	alex, sam := h.client(1), h.client(2)
	alex.signIn(o, "alex@example.com", same)
	alex.must(http.StatusOK, "PUT", "/v1/me/username", map[string]string{"username": "alex"}, nil)
	sam.signIn(o, "sam@example.com", same)
	sam.must(http.StatusOK, "PUT", "/v1/me/username", map[string]string{"username": "sam", "display_name": "Sam"}, nil)
	v := alex.createSpace("Team")
	base := "/v1/spaces/" + v.Space.ID.String()

	var found map[string]string
	alex.must(http.StatusOK, "GET", "/v1/users/SAM", nil, &found)
	if found["name"] != "Sam" {
		t.Fatalf("found %v", found)
	}
	if status := alex.do("GET", "/v1/users/sa", nil, nil); status != http.StatusNotFound {
		t.Fatalf("a prefix: %d", status)
	}
	if status := sam.do("POST", base+"/invitations/users", map[string]string{"username": "alex"}, nil); status != http.StatusNotFound {
		t.Fatalf("a stranger invites: %d", status)
	}
	alex.must(http.StatusCreated, "POST", base+"/invitations/users", map[string]string{"username": "sam"}, nil)

	var waiting []map[string]any
	sam.must(http.StatusOK, "GET", "/v1/me/invitations", nil, &waiting)
	if len(waiting) != 1 || waiting[0]["space_name"] != "Team" || waiting[0]["invited_by"] != "alex" {
		t.Fatalf("waiting %v", waiting)
	}
	var joined SpaceView
	sam.must(http.StatusOK, "POST", "/v1/me/invitations/"+v.Space.ID.String(), nil, &joined)
	if joined.Role != "member" || len(joined.Members) != 2 {
		t.Fatalf("joined %+v", joined)
	}
	if status := alex.do("POST", base+"/invitations/users", map[string]string{"username": "sam"}, nil); status != http.StatusConflict {
		t.Fatalf("invite a member: %d", status)
	}
	for _, m := range joined.Members {
		if m.Username == "" || len(m.Devices) != 1 {
			t.Fatalf("member %+v", m)
		}
	}
}

func TestAnAccountWithAUserNameCreatesSpaces(t *testing.T) {
	o := &outbox{}
	h := newHarness(t, Config{Mail: o})
	alex := h.client(1)
	alex.signIn(o, "alex@example.com", same)
	body := map[string]string{"name": "Team", "display_name": "alex"}
	if status := alex.do("POST", "/v1/spaces", body, nil); status != http.StatusForbidden {
		t.Fatalf("no user name yet: %d", status)
	}
	alex.must(http.StatusOK, "PUT", "/v1/me/username", map[string]string{"username": "alex"}, nil)
	alex.must(http.StatusCreated, "POST", "/v1/spaces", body, nil)

	// An operator's list restricts creation to the keys it names.
	h2 := newHarness(t, Config{Mail: o, SpaceCreators: []store.Key{h.client(9).key}})
	sam := h2.client(2)
	sam.signIn(o, "sam@example.com", same)
	sam.must(http.StatusOK, "PUT", "/v1/me/username", map[string]string{"username": "sam"}, nil)
	if status := sam.do("POST", "/v1/spaces", body, nil); status != http.StatusForbidden {
		t.Fatalf("not listed: %d", status)
	}
}
