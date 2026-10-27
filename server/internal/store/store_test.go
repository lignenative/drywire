package store

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/lignenative/drywire/server/internal/ledger"
)

type fakeClock struct{ t time.Time }

func (c *fakeClock) now() time.Time          { return c.t }
func (c *fakeClock) advance(d time.Duration) { c.t = c.t.Add(d) }

// testStore opens a fresh database and ledger in a temporary directory.
func testStore(t *testing.T) (*Store, *ledger.Ledger, *fakeClock) {
	t.Helper()
	dir := t.TempDir()
	l, err := ledger.Open(filepath.Join(dir, "deletions.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	clock := &fakeClock{t: time.Unix(1_800_000_000, 0)}
	s, err := Open(filepath.Join(dir, "drywire.db"), l, clock.now)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { s.Close(); l.Close() })
	return s, l, clock
}

func key(n byte) Key {
	var k Key
	for i := range k {
		k[i] = n
	}
	return k
}

func code(n byte) [CodeSize]byte {
	var c [CodeSize]byte
	for i := range c {
		c[i] = n
	}
	return c
}

func must[T any](t *testing.T) func(T, error) T {
	return func(v T, err error) T {
		t.Helper()
		if err != nil {
			t.Fatal(err)
		}
		return v
	}
}

func identity(t *testing.T, s *Store, k Key) Identity {
	t.Helper()
	return must[Identity](t)(s.IdentityByKey(context.Background(), k))
}

// space creates a space owned by key(1) ("alex") and returns it with its
// first room.
func space(t *testing.T, s *Store) (Space, Room) {
	t.Helper()
	sp, room, err := s.CreateSpace(context.Background(), key(1), "alex", "Team", "general", 3)
	if err != nil {
		t.Fatal(err)
	}
	return sp, room
}

// codes numbers the codes join makes, from 100 so they never meet the
// codes a test makes itself.
var codes byte = 100

// join makes the holder of k a member through a fresh invitation.
func join(t *testing.T, s *Store, sp Space, k Key, name string) Identity {
	t.Helper()
	ctx := context.Background()
	codes++
	c := code(codes)
	if _, err := s.CreateInvitation(ctx, sp.ID, identity(t, s, key(1)).ID, c, time.Hour, 1); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Redeem(ctx, c, k, name); err != nil {
		t.Fatal(err)
	}
	return identity(t, s, k)
}

func TestMigrationsRunOnceAndRefuseANewerSchema(t *testing.T) {
	dir := t.TempDir()
	l, _ := ledger.Open(filepath.Join(dir, "l"))
	defer l.Close()
	path := filepath.Join(dir, "db")
	s, err := Open(path, l, time.Now)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := s.db.Exec(`PRAGMA user_version = 99`); err != nil {
		t.Fatal(err)
	}
	s.Close()
	if _, err := Open(path, l, time.Now); err == nil {
		t.Fatal("a schema from a newer server was accepted")
	}
}

func TestTheDatabaseIsPrivate(t *testing.T) {
	dir := t.TempDir()
	l, _ := ledger.Open(filepath.Join(dir, "l"))
	defer l.Close()
	s, err := Open(filepath.Join(dir, "db"), l, time.Now)
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	for _, name := range []string{"db", "db-wal"} {
		info, err := os.Stat(filepath.Join(dir, name))
		if err != nil {
			t.Fatal(err)
		}
		if info.Mode().Perm() != 0o600 {
			t.Errorf("%s: mode %v", name, info.Mode())
		}
	}
}

func TestCleanName(t *testing.T) {
	for in, want := range map[string]string{" Alex ": "Alex", "Zoé": "Zoé", "12345678901234567890123456789012": "12345678901234567890123456789012"} {
		if got, err := CleanName(in); err != nil || got != want {
			t.Errorf("CleanName(%q) = %q, %v", in, got, err)
		}
	}
	for _, in := range []string{"", "   ", "a\nb", "a‮b", "\xff", "123456789012345678901234567890123"} {
		if _, err := CleanName(in); err == nil {
			t.Errorf("CleanName(%q) accepted", in)
		}
	}
}

func TestASpaceHasItsOwnerAndFirstRoom(t *testing.T) {
	s, _, _ := testStore(t)
	ctx := context.Background()
	sp, room := space(t, s)
	alex := identity(t, s, key(1))
	if alex.Name != "alex" {
		t.Fatalf("name %q", alex.Name)
	}
	if role, err := s.Role(ctx, sp.ID, alex.ID); err != nil || role != RoleOwner {
		t.Fatalf("role %q, %v", role, err)
	}
	rooms := must[[]Room](t)(s.Rooms(ctx, sp.ID))
	if len(rooms) != 1 || rooms[0] != room {
		t.Fatalf("rooms %+v", rooms)
	}
	if _, err := s.CreateRoom(ctx, sp.ID, "general"); !errors.Is(err, ErrNameTaken) {
		t.Fatalf("duplicate room name: %v", err)
	}
	// A second space by the same key reuses the identity and its name.
	if _, _, err := s.CreateSpace(ctx, key(1), "other name", "Second", "lobby", 3); err != nil {
		t.Fatal(err)
	}
	if got := must[[]Membership](t)(s.Memberships(ctx, alex.ID)); len(got) != 2 || identity(t, s, key(1)).Name != "alex" {
		t.Fatalf("memberships %+v", got)
	}
}

func TestInvitationsExpireAndRunOut(t *testing.T) {
	s, _, clock := testStore(t)
	ctx := context.Background()
	sp, _ := space(t, s)
	alex := identity(t, s, key(1))

	if _, err := s.CreateInvitation(ctx, sp.ID, alex.ID, code(7), time.Hour, 1); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Redeem(ctx, code(8), key(2), "sam"); !errors.Is(err, ErrInvalidCode) {
		t.Fatalf("unknown code: %v", err)
	}
	got, err := s.Redeem(ctx, code(7), key(2), "sam")
	if err != nil || got.ID != sp.ID {
		t.Fatalf("redeem: %+v, %v", got, err)
	}
	// A member using a code again does not use it up, but the next person
	// finds it used up.
	if _, err := s.Redeem(ctx, code(7), key(2), "sam"); !errors.Is(err, ErrInvalidCode) {
		t.Fatalf("used-up code: %v", err)
	}

	if _, err := s.CreateInvitation(ctx, sp.ID, alex.ID, code(9), time.Hour, 5); err != nil {
		t.Fatal(err)
	}
	clock.advance(time.Hour)
	if _, err := s.Redeem(ctx, code(9), key(3), "kim"); !errors.Is(err, ErrInvalidCode) {
		t.Fatalf("expired code: %v", err)
	}
	if _, err := s.IdentityByKey(ctx, key(3)); !errors.Is(err, ErrNotFound) {
		t.Fatalf("a refused redemption left an identity behind: %v", err)
	}
	if invs := must[[]Invitation](t)(s.Invitations(ctx, sp.ID)); len(invs) != 0 {
		t.Fatalf("unusable invitations listed: %+v", invs)
	}
}

func TestASpaceHasAMaximumNumberOfMembers(t *testing.T) {
	s, _, _ := testStore(t)
	ctx := context.Background()
	sp, _ := space(t, s) // at most 3 members
	join(t, s, sp, key(2), "sam")
	join(t, s, sp, key(3), "kim")
	if _, err := s.CreateInvitation(ctx, sp.ID, identity(t, s, key(1)).ID, code(40), time.Hour, 5); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Redeem(ctx, code(40), key(4), "lee"); !errors.Is(err, ErrSpaceFull) {
		t.Fatalf("fourth member: %v", err)
	}
}

func TestABanOutlivesTheMembershipAndTheIdentity(t *testing.T) {
	s, _, _ := testStore(t)
	ctx := context.Background()
	sp, _ := space(t, s)
	sam := join(t, s, sp, key(2), "sam")

	if err := s.Ban(ctx, sp.ID, key(2)); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Role(ctx, sp.ID, sam.ID); !errors.Is(err, ErrNotFound) {
		t.Fatalf("banned member still in the space: %v", err)
	}
	if _, err := s.DeleteIdentity(ctx, sam.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := s.CreateInvitation(ctx, sp.ID, identity(t, s, key(1)).ID, code(50), time.Hour, 5); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Redeem(ctx, code(50), key(2), "sam again"); !errors.Is(err, ErrBanned) {
		t.Fatalf("banned key joined again: %v", err)
	}
	if err := s.Unban(ctx, sp.ID, key(2)); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Redeem(ctx, code(50), key(2), "sam again"); err != nil {
		t.Fatalf("after unban: %v", err)
	}
}

func TestDeletingTheOwnerHandsTheSpaceOn(t *testing.T) {
	s, l, clock := testStore(t)
	ctx := context.Background()
	sp, _ := space(t, s)
	clock.advance(time.Second)
	sam := join(t, s, sp, key(2), "sam")
	clock.advance(time.Second)
	kim := join(t, s, sp, key(3), "kim")
	if err := s.SetRole(ctx, sp.ID, kim.ID, RoleAdmin); err != nil {
		t.Fatal(err)
	}
	// Alone in its space, alex's second space goes with them.
	solo, _, _ := s.CreateSpace(ctx, key(1), "alex", "Solo", "general", 3)

	d, err := s.DeleteIdentity(ctx, identity(t, s, key(1)).ID)
	if err != nil {
		t.Fatal(err)
	}
	if len(d.Spaces) != 1 || d.Spaces[0] != solo.ID || len(d.Memberships) != 2 {
		t.Fatalf("deleted %+v", d)
	}
	// The admin inherits, though sam joined first.
	if role, _ := s.Role(ctx, sp.ID, kim.ID); role != RoleOwner {
		t.Fatalf("kim is %q", role)
	}
	if role, _ := s.Role(ctx, sp.ID, sam.ID); role != RoleMember {
		t.Fatalf("sam is %q", role)
	}
	if _, err := s.Space(ctx, solo.ID); !errors.Is(err, ErrNotFound) {
		t.Fatalf("the empty space survived: %v", err)
	}
	entries := must[[]ledger.Entry](t)(l.Entries())
	kinds := ""
	for _, e := range entries {
		kinds += e.Kind + " "
	}
	if kinds != "identity space " {
		t.Fatalf("ledger: %s", kinds)
	}
	if entries[0].Identity != ledger.KeyHash(func() []byte { k := key(1); return k[:] }()) {
		t.Fatalf("the ledger names the identity by %q", entries[0].Identity)
	}
}

func TestInactiveIdentitiesArePurged(t *testing.T) {
	s, _, clock := testStore(t)
	ctx := context.Background()
	sp, _ := space(t, s)
	join(t, s, sp, key(2), "sam")

	clock.advance(InactiveAfter - time.Hour)
	if err := s.Touch(ctx, key(2)); err != nil {
		t.Fatal(err)
	}
	clock.advance(2 * time.Hour)
	p, err := s.PurgeInactive(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if p.Identities != 1 || p.Invitations != 1 {
		t.Fatalf("purged %+v", p)
	}
	if _, err := s.IdentityByKey(ctx, key(1)); !errors.Is(err, ErrNotFound) {
		t.Fatal("alex, inactive for a year, is still there")
	}
	// Sam logged in an hour before the year was up, and inherited the space.
	sam := identity(t, s, key(2))
	if role, _ := s.Role(ctx, sp.ID, sam.ID); role != RoleOwner {
		t.Fatalf("sam is %q", role)
	}
}

func TestExportHoldsTheIdentitysData(t *testing.T) {
	s, _, _ := testStore(t)
	ctx := context.Background()
	sp, _ := space(t, s)
	alex := identity(t, s, key(1))
	if _, err := s.CreateInvitation(ctx, sp.ID, alex.ID, code(60), time.Hour, 2); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Rename(ctx, alex.ID, "Alex B"); err != nil {
		t.Fatal(err)
	}
	e := must[Export](t)(s.Export(ctx, alex.ID))
	if e.Identity.Name != "Alex B" || e.Identity.PublicKey != key(1) {
		t.Fatalf("identity %+v", e.Identity)
	}
	if len(e.Memberships) != 1 || e.Memberships[0].Role != RoleOwner || e.Memberships[0].SpaceName != "Team" {
		t.Fatalf("memberships %+v", e.Memberships)
	}
	if len(e.Invitations) != 1 || e.Invitations[0].MaxUses != 2 {
		t.Fatalf("invitations %+v", e.Invitations)
	}
}

// A backup taken before deletions, restored afterwards, comes back with
// the deleted data; replaying the ledger deletes it again, and only it.
func TestReplayDeletesAgainWhatARestoreBroughtBack(t *testing.T) {
	ctx := context.Background()
	s, l, clock := testStore(t)
	sp, _ := space(t, s)
	sam := join(t, s, sp, key(2), "sam")
	kim := join(t, s, sp, key(3), "kim")
	solo, _, _ := s.CreateSpace(ctx, key(3), "kim", "Solo", "general", 3)

	// The "backup": a copy of the database at this point.
	backup := filepath.Join(t.TempDir(), "backup.db")
	if _, err := s.db.Exec(`VACUUM INTO ?`, backup); err != nil {
		t.Fatal(err)
	}

	clock.advance(time.Minute)
	if _, err := s.DeleteIdentity(ctx, sam.ID); err != nil {
		t.Fatal(err)
	}
	if err := s.RemoveMember(ctx, sp.ID, kim.ID); err != nil {
		t.Fatal(err)
	}
	if err := s.DeleteSpace(ctx, solo.ID); err != nil {
		t.Fatal(err)
	}
	// Sam comes back later with the same key: a new identity, which the
	// replay must leave alone.
	clock.advance(time.Minute)
	join(t, s, sp, key(2), "sam again")
	s.Close()

	restored, err := Open(backup, l, clock.now)
	if err != nil {
		t.Fatal(err)
	}
	defer restored.Close()
	if _, err := restored.IdentityByKey(ctx, key(2)); err != nil {
		t.Fatal("the backup should hold sam")
	}
	n, err := restored.Replay(ctx, must[[]ledger.Entry](t)(l.Entries()))
	if err != nil {
		t.Fatal(err)
	}
	if n != 3 {
		t.Fatalf("applied %d deletions, want 3", n)
	}
	if _, err := restored.IdentityByKey(ctx, key(2)); !errors.Is(err, ErrNotFound) {
		t.Fatal("sam's deleted identity came back with the restore")
	}
	kimAgain := must[Identity](t)(restored.IdentityByKey(ctx, key(3)))
	if _, err := restored.Role(ctx, sp.ID, kimAgain.ID); !errors.Is(err, ErrNotFound) {
		t.Fatal("kim's membership came back with the restore")
	}
	if _, err := restored.Space(ctx, solo.ID); !errors.Is(err, ErrNotFound) {
		t.Fatal("the deleted space came back with the restore")
	}
	if _, err := restored.Space(ctx, sp.ID); err != nil {
		t.Fatal("the team space must stay")
	}

	// The live database, with sam's new identity, is left alone.
	live, err := Open(filepath.Join(filepath.Dir(l.Path()), "drywire.db"), l, clock.now)
	if err != nil {
		t.Fatal(err)
	}
	defer live.Close()
	if n, err := live.Replay(ctx, must[[]ledger.Entry](t)(l.Entries())); err != nil || n != 0 {
		t.Fatalf("replay on the live database applied %d, %v", n, err)
	}
	if got := must[Identity](t)(live.IdentityByKey(ctx, key(2))); got.Name != "sam again" {
		t.Fatalf("sam's new identity: %+v", got)
	}
}
