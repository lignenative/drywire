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
// Everything that can fail happens on a temporary copy: reading the
// snapshot whole, checking it, migrating it and replaying the ledger. The
// current database is replaced only then, by a rename: a backup that does
// not decrypt, arrives cut short, or cannot be replayed leaves it as it
// was.
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
	name := tmp.Name()
	defer removeDatabase(name) // nothing left once renamed
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
	if err := store.CheckSnapshot(name); err != nil {
		return fmt.Errorf("the backup was not restored: %w", err)
	}

	_, statErr := os.Stat(ledgerPath)
	ledgerMissing := errors.Is(statErr, os.ErrNotExist)
	entries, applied, err := replay(name, ledgerPath)
	if err != nil {
		return fmt.Errorf("the backup was not restored: %w", err)
	}

	// The old database's write-ahead log belongs to the old file: left
	// behind, SQLite would apply it to the restored one. What it holds is
	// thrown away with the old database.
	for _, suffix := range []string{"-wal", "-shm"} {
		if err := os.Remove(db + suffix); err != nil && !errors.Is(err, os.ErrNotExist) {
			return err
		}
	}
	if err := os.Rename(name, db); err != nil {
		return err
	}
	fmt.Fprintf(log, "restored %s; deletions in the ledger: %d, applied again: %d\n", db, entries, applied)
	if ledgerMissing || entries == 0 {
		fmt.Fprintf(log, "warning: the deletion ledger at %s is missing or empty, so nothing deleted since this backup could be deleted again; on a new host, copy the old host's ledger there first and restore again\n", ledgerPath)
	}
	return nil
}

// replay migrates the database at path and replays the ledger on it. It
// leaves the database whole in its one file: closing the last connection
// checkpoints the write-ahead log into it.
func replay(path, ledgerPath string) (entries, applied int, err error) {
	led, err := ledger.Open(ledgerPath)
	if err != nil {
		return 0, 0, err
	}
	defer led.Close()
	all, err := led.Entries()
	if err != nil {
		return 0, 0, err
	}
	st, err := store.Open(path, led, time.Now)
	if err != nil {
		return 0, 0, err
	}
	applied, err = st.Replay(context.Background(), all)
	if cerr := st.Close(); err == nil {
		err = cerr
	}
	if err != nil {
		return 0, 0, fmt.Errorf("replaying the deletion ledger: %w", err)
	}
	if info, err := os.Stat(path + "-wal"); err == nil && info.Size() > 0 {
		return 0, 0, errors.New("the replayed copy kept changes in its write-ahead log")
	}
	return len(all), applied, nil
}

// removeDatabase removes a SQLite database and its companion files.
func removeDatabase(path string) {
	for _, suffix := range []string{"", "-wal", "-shm", "-journal"} {
		os.Remove(path + suffix)
	}
}

// removeLeftovers removes the temporary copies an interrupted backup or
// restore left beside the database (a killed process cannot clean up).
// They hold personal data outside the retention rules and the ledger's
// reach, so the server removes them at every start.
func removeLeftovers(db string) int {
	dir := filepath.Dir(db)
	var found []string
	for _, pattern := range []string{".backup-*.db", ".restore-*.db"} {
		m, _ := filepath.Glob(filepath.Join(dir, pattern))
		found = append(found, m...)
	}
	for _, f := range found {
		removeDatabase(f)
	}
	return len(found)
}
