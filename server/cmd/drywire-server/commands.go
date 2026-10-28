package main

import (
	"context"
	"crypto/tls"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/netip"
	"os"
	"path/filepath"
	"time"

	"github.com/lignenative/drywire/server/internal/ledger"
	"github.com/lignenative/drywire/server/internal/store"
)

// The operator's commands. The release image is built from scratch, with
// no shell, curl or sqlite3 in it, so the binary itself checks its health
// and moves its database in and out (ADR 0025). They read the same
// DRYWIRE_* variables as the server.
const commandsUsage = `Usage: drywire-server [command]

Without a command, run the relay and the control plane.

Commands:
  health    Exit 0 if the control plane on DRYWIRE_HTTP_ADDR answers
  backup    Write a consistent copy of the database to standard output,
            while the server runs
  restore   Replace the database with a copy read from standard input,
            then replay the deletion ledger; the server must be stopped
`

// runCommand runs one operator command, its output on stdout and its
// messages on stderr.
func runCommand(name string, stdin io.Reader, stdout, stderr io.Writer) error {
	switch name {
	case "health":
		return health()
	case "backup":
		return backup(stdout)
	case "restore":
		return restore(stdin, stderr)
	case "help", "-h", "--help":
		fmt.Fprint(stdout, commandsUsage)
		return nil
	}
	fmt.Fprint(stderr, commandsUsage)
	return fmt.Errorf("unknown command %q", name)
}

// health asks the control plane for /healthz on its own address.
func health() error {
	addr := envOr("DRYWIRE_HTTP_ADDR", defaultAddr)
	host, port, err := net.SplitHostPort(addr)
	if err != nil {
		return fmt.Errorf("DRYWIRE_HTTP_ADDR: %w", err)
	}
	// A server listening on every address answers on loopback too.
	if ip, err := netip.ParseAddr(host); host == "" || (err == nil && ip.IsUnspecified()) {
		host = "127.0.0.1"
	}
	client := &http.Client{Timeout: 3 * time.Second}
	scheme := "http"
	if os.Getenv("DRYWIRE_TLS_CERT") != "" {
		scheme = "https"
		// The certificate names the public host, not the loopback address
		// asked here, and a scratch image has no CA certificates. Nothing
		// is sent but the request for /healthz, so not checking it costs
		// nothing.
		client.Transport = &http.Transport{TLSClientConfig: &tls.Config{InsecureSkipVerify: true}}
	}
	resp, err := client.Get(scheme + "://" + net.JoinHostPort(host, port) + "/healthz")
	if err != nil {
		return err
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("/healthz answered %s", resp.Status)
	}
	return nil
}

// backup writes a snapshot of the database to w. The snapshot goes
// through a temporary file beside the database, readable by the server's
// user only, removed before returning.
func backup(w io.Writer) error {
	db := envOr("DRYWIRE_DB", "drywire.db")
	tmp, err := os.CreateTemp(filepath.Dir(db), ".backup-*.db")
	if err != nil {
		return err
	}
	tmp.Close()
	defer os.Remove(tmp.Name())
	if err := store.Snapshot(db, tmp.Name()); err != nil {
		return err
	}
	f, err := os.Open(tmp.Name())
	if err != nil {
		return err
	}
	defer f.Close()
	_, err = io.Copy(w, f)
	return err
}

// restore replaces the database with the snapshot read from r, then
// replays the deletion ledger on it, so that nothing deleted since the
// backup comes back (spec section 9, ADR 0019). The server replays it at
// every start anyway; doing it here reports what the replay did, and the
// database is never left on disk with deleted rows back in it.
//
// The current database is only replaced once the snapshot has been read
// whole and checked: a backup that fails to decrypt or arrives cut short
// leaves it untouched.
func restore(r io.Reader, log io.Writer) error {
	if health() == nil {
		return fmt.Errorf("a server answers on %s: stop it before restoring", envOr("DRYWIRE_HTTP_ADDR", defaultAddr))
	}
	db := envOr("DRYWIRE_DB", "drywire.db")
	ledgerPath := envOr("DRYWIRE_LEDGER", db+".deletions")

	tmp, err := os.CreateTemp(filepath.Dir(db), ".restore-*.db")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name()) // fails harmlessly once renamed
	_, err = io.Copy(tmp, r)
	if err == nil {
		err = tmp.Sync()
	}
	if cerr := tmp.Close(); err == nil {
		err = cerr
	}
	if err != nil {
		return fmt.Errorf("reading the backup: %w", err)
	}
	if err := store.CheckSnapshot(tmp.Name()); err != nil {
		return fmt.Errorf("the backup was not restored: %w", err)
	}

	_, statErr := os.Stat(ledgerPath)
	ledgerMissing := errors.Is(statErr, os.ErrNotExist)

	// The old database's write-ahead log belongs to the old file: left
	// behind, SQLite would apply it to the restored one. What it holds is
	// being thrown away with the old database anyway.
	for _, suffix := range []string{"-wal", "-shm"} {
		if err := os.Remove(db + suffix); err != nil && !errors.Is(err, os.ErrNotExist) {
			return err
		}
	}
	if err := os.Rename(tmp.Name(), db); err != nil {
		return err
	}

	led, err := ledger.Open(ledgerPath)
	if err != nil {
		return err
	}
	defer led.Close()
	st, err := store.Open(db, led, time.Now)
	if err != nil {
		return err
	}
	defer st.Close()
	entries, err := led.Entries()
	if err != nil {
		return err
	}
	applied, err := st.Replay(context.Background(), entries)
	if err != nil {
		return fmt.Errorf("replaying the deletion ledger: %w", err)
	}
	fmt.Fprintf(log, "restored %s; deletions in the ledger: %d, applied again: %d\n", db, len(entries), applied)
	if ledgerMissing {
		fmt.Fprintf(log, "warning: there was no deletion ledger at %s, so nothing deleted since this backup could be deleted again; on a new host, copy the old host's ledger there first\n", ledgerPath)
	}
	return nil
}
