package store

import (
	"database/sql"
	"errors"
	"fmt"
	"os"
	"strings"
)

// Snapshot writes a consistent copy of the database at path to dst while a
// server may be using it: VACUUM INTO reads the database in one
// transaction, so the copy is never caught halfway through a change, and
// the server keeps answering meanwhile. dst must not exist or be empty;
// the caller creates it, readable by its owner only. The database must
// already exist: a backup never creates an empty one by mistake.
func Snapshot(path, dst string) error {
	if strings.ContainsAny(path, "?#") {
		return fmt.Errorf("database path %q: ? and # are not allowed", path)
	}
	if _, err := os.Stat(path); err != nil {
		return err
	}
	// No journal_mode pragma: the copy only reads, and must not change
	// how the running server keeps the file.
	db, err := sql.Open("sqlite", "file:"+path+"?_pragma=busy_timeout(5000)")
	if err != nil {
		return err
	}
	defer db.Close()
	if _, err := db.Exec(`VACUUM INTO ?`, dst); err != nil {
		return fmt.Errorf("snapshot of %s: %w", path, err)
	}
	return nil
}

// CheckSnapshot tells whether the file at path is a database this server
// can take in place of its own: an intact SQLite file whose schema is not
// newer than this server knows. It changes nothing in the file.
func CheckSnapshot(path string) error {
	header := make([]byte, 16)
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	_, err = f.Read(header)
	f.Close()
	// Without this check, SQLite would treat an empty or foreign file as a
	// new database and the checks below would pass.
	if err != nil || string(header) != "SQLite format 3\x00" {
		return errors.New("not a SQLite database (wrong file, or a backup that did not decrypt?)")
	}

	db, err := sql.Open("sqlite", "file:"+path+"?mode=ro")
	if err != nil {
		return err
	}
	defer db.Close()
	var result string
	if err := db.QueryRow(`PRAGMA integrity_check`).Scan(&result); err != nil {
		return fmt.Errorf("damaged database: %w", err)
	}
	if result != "ok" {
		return fmt.Errorf("damaged database: %s", result)
	}
	var version int
	if err := db.QueryRow(`PRAGMA user_version`).Scan(&version); err != nil {
		return err
	}
	if version > len(migrations) {
		return fmt.Errorf("schema version %d is newer than this server knows (%d): restore with a newer server", version, len(migrations))
	}
	return nil
}
