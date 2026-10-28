package main

import (
	"bytes"
	"context"
	"errors"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/lignenative/drywire/server/internal/ledger"
	"github.com/lignenative/drywire/server/internal/store"
)

// freeAddr returns a loopback address where nothing listens.
func freeAddr(t *testing.T) string {
	t.Helper()
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	addr := l.Addr().String()
	l.Close()
	return addr
}

func TestHealthAsksTheControlPlane(t *testing.T) {
	t.Setenv("DRYWIRE_HTTP_ADDR", freeAddr(t))
	if err := health(); err == nil {
		t.Fatal("healthy with no server")
	}

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/healthz" {
			http.NotFound(w, r)
		}
	}))
	defer srv.Close()
	_, port, _ := net.SplitHostPort(srv.Listener.Addr().String())
	// Listening on every address: asked through loopback.
	t.Setenv("DRYWIRE_HTTP_ADDR", "0.0.0.0:"+port)
	if err := health(); err != nil {
		t.Fatalf("unhealthy with a server: %v", err)
	}
}

// A backup taken before a deletion, restored after it, does not bring the
// deleted space back: restore replays the ledger.
func TestRestoreReplaysTheLedgerOnTheBackup(t *testing.T) {
	dir := t.TempDir()
	db := filepath.Join(dir, "drywire.db")
	t.Setenv("DRYWIRE_DB", db)
	t.Setenv("DRYWIRE_LEDGER", "")
	t.Setenv("DRYWIRE_HTTP_ADDR", freeAddr(t))
	ctx := context.Background()

	led, err := ledger.Open(db + ".deletions")
	if err != nil {
		t.Fatal(err)
	}
	// The deletion must be strictly newer than the rows it deletes.
	now := time.Now().Add(-time.Hour)
	st, err := store.Open(db, led, func() time.Time { return now })
	if err != nil {
		t.Fatal(err)
	}
	var owner store.Key
	owner[0] = 1
	kept, _, err := st.CreateSpace(ctx, owner, "alex", "Kept", "general", 10)
	if err != nil {
		t.Fatal(err)
	}
	gone, _, err := st.CreateSpace(ctx, owner, "alex", "Gone", "general", 10)
	if err != nil {
		t.Fatal(err)
	}

	// The backup runs while the store is open, as beside a running server.
	var snapshot bytes.Buffer
	if err := runCommand("backup", nil, &snapshot, os.Stderr); err != nil {
		t.Fatal(err)
	}
	now = now.Add(time.Minute)
	if err := st.DeleteSpace(ctx, gone.ID); err != nil {
		t.Fatal(err)
	}
	st.Close()
	led.Close()

	var messages bytes.Buffer
	if err := runCommand("restore", &snapshot, os.Stdout, &messages); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(messages.String(), "applied again: 1") {
		t.Errorf("restore said %q", messages.String())
	}

	led, err = ledger.Open(db + ".deletions")
	if err != nil {
		t.Fatal(err)
	}
	defer led.Close()
	st, err = store.Open(db, led, time.Now)
	if err != nil {
		t.Fatal(err)
	}
	defer st.Close()
	if _, err := st.Space(ctx, kept.ID); err != nil {
		t.Errorf("the kept space is gone: %v", err)
	}
	if _, err := st.Space(ctx, gone.ID); !errors.Is(err, store.ErrNotFound) {
		t.Errorf("the deleted space came back: %v", err)
	}
	leftovers, _ := filepath.Glob(filepath.Join(dir, ".*"))
	if len(leftovers) > 0 {
		t.Errorf("temporary files left behind: %v", leftovers)
	}
}

func TestRestoreLeavesTheDatabaseAloneOnBadInput(t *testing.T) {
	dir := t.TempDir()
	db := filepath.Join(dir, "drywire.db")
	t.Setenv("DRYWIRE_DB", db)
	t.Setenv("DRYWIRE_LEDGER", "")
	t.Setenv("DRYWIRE_HTTP_ADDR", freeAddr(t))
	if err := os.WriteFile(db, []byte("the current database"), 0o600); err != nil {
		t.Fatal(err)
	}

	err := runCommand("restore", strings.NewReader("age-encryption.org/v1 …"), os.Stdout, os.Stderr)
	if err == nil {
		t.Fatal("restored an encrypted file")
	}
	if got, _ := os.ReadFile(db); string(got) != "the current database" {
		t.Errorf("the database changed: %q", got)
	}
}

func TestRestoreRefusesWhileAServerAnswers(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {}))
	defer srv.Close()
	t.Setenv("DRYWIRE_HTTP_ADDR", srv.Listener.Addr().String())
	t.Setenv("DRYWIRE_DB", filepath.Join(t.TempDir(), "drywire.db"))
	err := runCommand("restore", strings.NewReader(""), os.Stdout, os.Stderr)
	if err == nil || !strings.Contains(err.Error(), "stop it") {
		t.Fatalf("restore with a server running: %v", err)
	}
}

// A backup that cannot be replayed (here, a ledger the server cannot
// open, as when it belongs to another user) leaves the current database
// in place.
func TestRestoreLeavesTheDatabaseAloneWhenTheReplayFails(t *testing.T) {
	dir := t.TempDir()
	db := filepath.Join(dir, "drywire.db")
	t.Setenv("DRYWIRE_DB", db)
	t.Setenv("DRYWIRE_HTTP_ADDR", freeAddr(t))
	t.Setenv("DRYWIRE_LEDGER", filepath.Join(dir, "deletions"))
	led, err := ledger.Open(filepath.Join(dir, "deletions"))
	if err != nil {
		t.Fatal(err)
	}
	st, err := store.Open(db, led, time.Now)
	if err != nil {
		t.Fatal(err)
	}
	var snapshot bytes.Buffer
	if err := runCommand("backup", nil, &snapshot, os.Stderr); err != nil {
		t.Fatal(err)
	}
	st.Close()
	led.Close()
	if err := os.WriteFile(db, []byte("the current database"), 0o600); err != nil {
		t.Fatal(err)
	}

	t.Setenv("DRYWIRE_LEDGER", dir) // a directory: opening it fails
	if err := runCommand("restore", &snapshot, os.Stdout, os.Stderr); err == nil {
		t.Fatal("restored without replaying the ledger")
	}
	if got, _ := os.ReadFile(db); string(got) != "the current database" {
		t.Errorf("the database changed: %q", got)
	}
	if leftovers, _ := filepath.Glob(filepath.Join(dir, ".restore-*")); len(leftovers) > 0 {
		t.Errorf("temporary files left behind: %v", leftovers)
	}
}

func TestRestoreWarnsWithAnEmptyLedger(t *testing.T) {
	dir := t.TempDir()
	db := filepath.Join(dir, "drywire.db")
	t.Setenv("DRYWIRE_DB", db)
	t.Setenv("DRYWIRE_LEDGER", "")
	t.Setenv("DRYWIRE_HTTP_ADDR", freeAddr(t))
	led, err := ledger.Open(db + ".deletions")
	if err != nil {
		t.Fatal(err)
	}
	st, err := store.Open(db, led, time.Now)
	if err != nil {
		t.Fatal(err)
	}
	var snapshot bytes.Buffer
	if err := runCommand("backup", nil, &snapshot, os.Stderr); err != nil {
		t.Fatal(err)
	}
	st.Close()
	led.Close()

	var messages bytes.Buffer
	if err := runCommand("restore", &snapshot, os.Stdout, &messages); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(messages.String(), "missing or empty") {
		t.Errorf("no warning: %q", messages.String())
	}
}

func TestLeftoverCopiesAreRemoved(t *testing.T) {
	dir := t.TempDir()
	db := filepath.Join(dir, "drywire.db")
	keep := []string{db, db + "-wal", db + ".deletions"}
	gone := []string{".backup-123.db", ".restore-456.db", ".restore-456.db-wal"}
	for _, f := range append(slices.Clone(keep), gone...) {
		if err := os.WriteFile(filepath.Join(dir, filepath.Base(f)), nil, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	if n := removeLeftovers(db); n != 2 {
		t.Errorf("removed %d copies, want 2", n)
	}
	for _, f := range keep {
		if _, err := os.Stat(f); err != nil {
			t.Errorf("%s was removed", filepath.Base(f))
		}
	}
	for _, f := range gone {
		if _, err := os.Stat(filepath.Join(dir, f)); err == nil {
			t.Errorf("%s is still there", f)
		}
	}
}
