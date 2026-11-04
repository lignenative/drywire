package store

import (
	"context"
	"errors"
	"testing"
	"time"
)

func TestAddressesAndUserNamesAreNormalized(t *testing.T) {
	for in, want := range map[string]string{" Alex@Example.COM ": "alex@example.com", "a.b+c@x.example.org": "a.b+c@x.example.org"} {
		if got, err := NormalizeEmail(in); err != nil || got != want {
			t.Errorf("%q: %q %v", in, got, err)
		}
	}
	for _, bad := range []string{"", "alex", "@example.com", "alex@", "alex@localhost", "a b@example.com", "a@b.c\r\nBcc: x"} {
		if _, err := NormalizeEmail(bad); err == nil {
			t.Errorf("%q accepted", bad)
		}
	}
	for in, want := range map[string]string{"Alex": "alex", "sam_92": "sam_92", "a.b": "a.b"} {
		if got, err := NormalizeUsername(in); err != nil || got != want {
			t.Errorf("%q: %q %v", in, got, err)
		}
	}
	for _, bad := range []string{"a", ".alex", "alex.", "a..b", "alex!", "é", "abcdefghijklmnopqrstuvwxyz0123456"} {
		if _, err := NormalizeUsername(bad); err == nil {
			t.Errorf("%q accepted", bad)
		}
	}
}

func TestACodeWorksOnceWithinItsTimeAndTries(t *testing.T) {
	s, _, clock := testStore(t)
	ctx := context.Background()
	const email = "alex@example.com"

	code, err := s.NewEmailCode(ctx, email)
	if err != nil || len(code) != 6 {
		t.Fatalf("code %q %v", code, err)
	}
	if _, err := s.NewEmailCode(ctx, email); !errors.Is(err, ErrTooSoon) {
		t.Fatalf("a second code at once: %v", err)
	}
	var wrong WrongCodeError
	if err := s.UseEmailCode(ctx, email, "000000x"); !errors.As(err, &wrong) || wrong.Left != CodeTries-1 {
		t.Fatalf("wrong code: %v", err)
	}
	if err := s.UseEmailCode(ctx, email, code); err != nil {
		t.Fatalf("right code: %v", err)
	}
	if err := s.UseEmailCode(ctx, email, code); !errors.Is(err, ErrNoCode) {
		t.Fatalf("used twice: %v", err)
	}

	// Five wrong tries spend it, even the right code after.
	clock.advance(time.Minute)
	code, _ = s.NewEmailCode(ctx, email)
	for i := 0; i < CodeTries-1; i++ {
		if err := s.UseEmailCode(ctx, email, "bad"); !errors.As(err, &wrong) {
			t.Fatalf("try %d: %v", i, err)
		}
	}
	if err := s.UseEmailCode(ctx, email, "bad"); !errors.Is(err, ErrNoCode) {
		t.Fatalf("last try: %v", err)
	}
	if err := s.UseEmailCode(ctx, email, code); !errors.Is(err, ErrNoCode) {
		t.Fatalf("after the last try: %v", err)
	}

	// Ten minutes and it is gone.
	clock.advance(time.Minute)
	code, _ = s.NewEmailCode(ctx, email)
	clock.advance(CodeLifetime)
	if err := s.UseEmailCode(ctx, email, code); !errors.Is(err, ErrNoCode) {
		t.Fatalf("expired: %v", err)
	}
}

func TestAnAddressIsOneAccountAcrossDevices(t *testing.T) {
	s, _, _ := testStore(t)
	ctx := context.Background()

	pc, created, err := s.SignIn(ctx, "alex@example.com", key(1))
	if err != nil || !created {
		t.Fatalf("sign up: %v %v", created, err)
	}
	if _, err := s.SetUsername(ctx, pc.ID, "Alex", ""); err != nil {
		t.Fatal(err)
	}
	phone, created, err := s.SignIn(ctx, "alex@example.com", key(2))
	if err != nil || created || phone.ID != pc.ID || phone.Username != "alex" || phone.Name != "alex" {
		t.Fatalf("second device: %+v %v %v", phone, created, err)
	}
	if again, _, err := s.SignIn(ctx, "alex@example.com", key(2)); err != nil || again.ID != pc.ID {
		t.Fatalf("again: %v", err)
	}
	if id, err := s.IdentityByKey(ctx, key(2)); err != nil || id.ID != pc.ID {
		t.Fatalf("by the phone's key: %v", err)
	}
	// Sam's address cannot take Alex's phone.
	if _, _, err := s.SignIn(ctx, "sam@example.com", key(2)); !errors.Is(err, ErrDeviceOwned) {
		t.Fatalf("someone else's device: %v", err)
	}
	devices := must[[]AccountDevice](t)(s.Devices(ctx, pc.ID))
	if len(devices) != 2 {
		t.Fatalf("devices %+v", devices)
	}
	if err := s.RemoveDevice(ctx, pc.ID, key(2)); err != nil {
		t.Fatal(err)
	}
	if _, err := s.IdentityByKey(ctx, key(2)); !errors.Is(err, ErrNotFound) {
		t.Fatalf("a removed device: %v", err)
	}

	// A user name is someone's: the next one gets suggestions.
	sam, _, _ := s.SignIn(ctx, "sam@example.com", key(3))
	var taken UsernameTakenError
	if _, err := s.SetUsername(ctx, sam.ID, "ALEX", "Sam"); !errors.As(err, &taken) || len(taken.Suggestions) != 2 {
		t.Fatalf("taken: %v", err)
	}
	if _, err := s.SetUsername(ctx, sam.ID, taken.Suggestions[0], "Sam"); err != nil {
		t.Fatalf("a suggestion: %v", err)
	}

	// A key-only account from before ADR 0055 keeps its spaces.
	sp, _ := space(t, s)
	old := join(t, s, sp, key(4), "kim")
	kim, created, err := s.SignIn(ctx, "kim@example.com", key(4))
	if err != nil || created || kim.ID != old.ID {
		t.Fatalf("an old account: %+v %v %v", kim, created, err)
	}
}

func TestAnInvitationByUserNameWaitsForItsPerson(t *testing.T) {
	s, _, clock := testStore(t)
	ctx := context.Background()
	sp, _ := space(t, s)
	alex := identity(t, s, key(1))
	sam, _, _ := s.SignIn(ctx, "sam@example.com", key(2))
	if _, err := s.SetUsername(ctx, sam.ID, "sam", "Sam"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.SetUsername(ctx, alex.ID, "alex", ""); err != nil {
		t.Fatal(err)
	}

	if _, err := s.InviteUser(ctx, sp.ID, alex.ID, "nobody"); !errors.Is(err, ErrNotFound) {
		t.Fatalf("unknown: %v", err)
	}
	if _, err := s.InviteUser(ctx, sp.ID, alex.ID, "alex"); !errors.Is(err, ErrAlreadyMember) {
		t.Fatalf("a member: %v", err)
	}
	if _, err := s.InviteUser(ctx, sp.ID, alex.ID, "Sam"); err != nil {
		t.Fatal(err)
	}
	waiting := must[[]UserInvitation](t)(s.UserInvitations(ctx, sam.ID))
	if len(waiting) != 1 || waiting[0].SpaceName != "Team" || waiting[0].InvitedBy != "alex" {
		t.Fatalf("waiting %+v", waiting)
	}
	if err := s.AcceptInvitation(ctx, sp.ID, sam.ID); err != nil {
		t.Fatal(err)
	}
	if role, err := s.Role(ctx, sp.ID, sam.ID); err != nil || role != RoleMember {
		t.Fatalf("role %v %v", role, err)
	}
	if err := s.AcceptInvitation(ctx, sp.ID, sam.ID); !errors.Is(err, ErrNotFound) {
		t.Fatalf("accepted twice: %v", err)
	}
	members := must[[]Member](t)(s.Members(ctx, sp.ID))
	if len(members) != 2 || members[1].Username != "sam" || len(members[1].Devices) != 1 || members[1].Devices[0] != key(2) {
		t.Fatalf("members %+v", members)
	}

	// Declined, or past its 30 days, it is gone.
	kim, _, _ := s.SignIn(ctx, "kim@example.com", key(3))
	_, _ = s.SetUsername(ctx, kim.ID, "kim", "")
	_, _ = s.InviteUser(ctx, sp.ID, alex.ID, "kim")
	if err := s.DeclineInvitation(ctx, sp.ID, kim.ID); err != nil {
		t.Fatal(err)
	}
	_, _ = s.InviteUser(ctx, sp.ID, alex.ID, "kim")
	clock.advance(InvitationLifetime)
	if err := s.AcceptInvitation(ctx, sp.ID, kim.ID); !errors.Is(err, ErrNotFound) {
		t.Fatalf("expired: %v", err)
	}
	if waiting := must[[]UserInvitation](t)(s.UserInvitations(ctx, kim.ID)); len(waiting) != 0 {
		t.Fatalf("expired still listed: %+v", waiting)
	}

	// The export holds the address, the devices and what waits.
	e := must[Export](t)(s.Export(ctx, sam.ID))
	if e.Email != "sam@example.com" || len(e.Devices) != 1 {
		t.Fatalf("export %+v", e)
	}
}
