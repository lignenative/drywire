package store

import (
	"context"
	"errors"
	"testing"
	"time"
)

func TestCreatorCodesReadAsTheOtherCodesDo(t *testing.T) {
	var c CreatorCode
	for i := range c {
		c[i] = byte(i)
	}
	// The same vector as vault codes (drywire-proto's vault.rs, vault.test.ts).
	if got := c.String(); got != "000G-40R4-0M30-E209-185G-R38E-1W" {
		t.Fatalf("String() = %s", got)
	}
	for _, typed := range []string{"000G-40R4-0M30-E209-185G-R38E-1W", "ooog 40r4 om30 e209 185g r38e 1w"} {
		if back, err := ParseCreatorCode(typed); err != nil || back != c {
			t.Fatalf("parse %q: %x %v", typed, back, err)
		}
	}
	for _, bad := range []string{"", "000G", "UUUU-UUUU-UUUU-UUUU-UUUU-UUUU-UU", "ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZ"} {
		if _, err := ParseCreatorCode(bad); !errors.Is(err, ErrBadCreatorCode) {
			t.Fatalf("parse %q: %v", bad, err)
		}
	}
	random := NewCreatorCode()
	if back, err := ParseCreatorCode(random.String()); err != nil || back != random {
		t.Fatalf("round trip: %v", err)
	}
}

func TestACreatorCodeCreatesOneSpaceOnly(t *testing.T) {
	s, _, clock := testStore(t)
	ctx := context.Background()
	code := NewCreatorCode()
	if _, err := s.AddCreatorCode(ctx, code, time.Hour); err != nil {
		t.Fatal(err)
	}
	// A bad name is refused before the code is spent.
	if _, _, err := s.CreateSpaceWithCode(ctx, code, key(1), "alex", "", "general", 50); !errors.Is(err, ErrBadName) {
		t.Fatalf("bad name: %v", err)
	}
	if _, _, err := s.CreateSpaceWithCode(ctx, code, key(1), "alex", "Team", "general", 50); err != nil {
		t.Fatal(err)
	}
	if _, _, err := s.CreateSpaceWithCode(ctx, code, key(2), "sam", "Other", "general", 50); !errors.Is(err, ErrInvalidCreatorCode) {
		t.Fatalf("used twice: %v", err)
	}
	if _, _, err := s.CreateSpaceWithCode(ctx, NewCreatorCode(), key(2), "sam", "Other", "general", 50); !errors.Is(err, ErrInvalidCreatorCode) {
		t.Fatalf("unknown: %v", err)
	}

	late := NewCreatorCode()
	if _, err := s.AddCreatorCode(ctx, late, time.Hour); err != nil {
		t.Fatal(err)
	}
	clock.advance(time.Hour)
	if _, _, err := s.CreateSpaceWithCode(ctx, late, key(2), "sam", "Other", "general", 50); !errors.Is(err, ErrInvalidCreatorCode) {
		t.Fatalf("expired: %v", err)
	}
	if _, err := s.PurgeInactive(ctx); err != nil {
		t.Fatal(err)
	}
	var left int
	if err := s.db.QueryRowContext(ctx, `SELECT count(*) FROM creator_codes`).Scan(&left); err != nil || left != 0 {
		t.Fatalf("codes left after the purge: %d %v", left, err)
	}
}
