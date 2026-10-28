package store

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/lignenative/drywire/server/internal/ledger"
)

func TestASnapshotTakenWhileOpenHoldsEveryCommittedChange(t *testing.T) {
	s, l, clock := testStore(t)
	sp, _ := space(t, s)
	dst := filepath.Join(t.TempDir(), "copy.db")

	// The store stays open, as the running server keeps it.
	if err := Snapshot(s.path(t), dst); err != nil {
		t.Fatal(err)
	}
	if err := CheckSnapshot(dst); err != nil {
		t.Fatalf("CheckSnapshot of a fresh snapshot: %v", err)
	}
	copied, err := Open(dst, l, clock.now)
	if err != nil {
		t.Fatal(err)
	}
	defer copied.Close()
	if _, err := copied.Space(context.Background(), sp.ID); err != nil {
		t.Fatalf("the space is not in the snapshot: %v", err)
	}
}

func TestSnapshotNeverCreatesADatabase(t *testing.T) {
	dir := t.TempDir()
	err := Snapshot(filepath.Join(dir, "missing.db"), filepath.Join(dir, "copy.db"))
	if err == nil {
		t.Fatal("a snapshot of a missing database succeeded")
	}
	if _, err := os.Stat(filepath.Join(dir, "missing.db")); !os.IsNotExist(err) {
		t.Fatal("the snapshot created the database it was asked to copy")
	}
}

func TestCheckSnapshotRefusesWhatItCannotRestore(t *testing.T) {
	dir := t.TempDir()
	write := func(name string, data []byte) string {
		p := filepath.Join(dir, name)
		if err := os.WriteFile(p, data, 0o600); err != nil {
			t.Fatal(err)
		}
		return p
	}

	if err := CheckSnapshot(write("empty.db", nil)); err == nil {
		t.Error("an empty file passed")
	}
	if err := CheckSnapshot(write("age.db", []byte("age-encryption.org/v1\n-> X25519 …"))); err == nil {
		t.Error("an encrypted file passed")
	}

	// A database from a newer server.
	s, _, _ := testStore(t)
	newer := filepath.Join(dir, "newer.db")
	if err := Snapshot(s.path(t), newer); err != nil {
		t.Fatal(err)
	}
	l, err := ledger.Open(filepath.Join(dir, "deletions"))
	if err != nil {
		t.Fatal(err)
	}
	defer l.Close()
	n, err := Open(newer, l, time.Now)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := n.db.Exec(`PRAGMA user_version = 999`); err != nil {
		t.Fatal(err)
	}
	n.Close()
	if err := CheckSnapshot(newer); err == nil || !strings.Contains(err.Error(), "newer") {
		t.Errorf("a database from a newer server: %v", err)
	}
}

// path returns the database file of a store opened by testStore.
func (s *Store) path(t *testing.T) string {
	t.Helper()
	var p string
	if err := s.db.QueryRow(`SELECT file FROM pragma_database_list WHERE name = 'main'`).Scan(&p); err != nil {
		t.Fatal(err)
	}
	return p
}
