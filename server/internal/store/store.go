// Package store keeps the control plane's lasting state in SQLite:
// identities, spaces, rooms, members and their roles, bans and invitation
// codes (docs/spec.md, section 8).
//
// It knows nothing about HTTP or about who may do what: the api package
// checks permissions, then calls one method here per change. Every method
// that changes several rows does it in one transaction.
//
// Deleting personal data goes through the deletion ledger first (package
// ledger), so that a later restore of an older backup can be corrected by
// Replay.
//
// SQLite comes from modernc.org/sqlite, a translation of SQLite to pure Go:
// the server stays one static binary, built without cgo.
package store

import (
	"context"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"os"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	"github.com/lignenative/drywire/server/internal/ledger"
	_ "modernc.org/sqlite" // registers the "sqlite" driver
)

// Errors the api package turns into HTTP statuses.
var (
	ErrNotFound    = errors.New("not found")
	ErrNameTaken   = errors.New("a room with this name already exists in the space")
	ErrBanned      = errors.New("banned from this space")
	ErrSpaceFull   = errors.New("the space has reached its maximum number of members")
	ErrInvalidCode = errors.New("this invitation is unknown, expired or used up")
	ErrBadName     = errors.New("a name must be 1 to 32 characters, without control characters")
)

// Role is a member's role in a space.
type Role string

const (
	// RoleOwner can do everything, including deleting the space. A space
	// has exactly one owner.
	RoleOwner Role = "owner"
	// RoleAdmin manages rooms, invitations and members, except other
	// admins and the owner.
	RoleAdmin Role = "admin"
	// RoleMember joins rooms and talks.
	RoleMember Role = "member"
)

// Rank orders roles: a higher rank may act on a lower one.
func (r Role) Rank() int {
	switch r {
	case RoleOwner:
		return 3
	case RoleAdmin:
		return 2
	case RoleMember:
		return 1
	}
	return 0
}

// Key is an Ed25519 public key. It reads and writes as lowercase hex.
type Key [32]byte

func (k Key) String() string                { return hex.EncodeToString(k[:]) }
func (k Key) MarshalText() ([]byte, error)  { return []byte(k.String()), nil }
func (k *Key) UnmarshalText(b []byte) error { return decodeHex(k[:], b, "public key") }

// ID identifies a space, a room or an invitation: 16 random bytes, written
// as lowercase hex. A room's ID is the room_id of the key derivation
// (protocol section 8.1).
type ID [16]byte

func (id ID) String() string                { return hex.EncodeToString(id[:]) }
func (id ID) MarshalText() ([]byte, error)  { return []byte(id.String()), nil }
func (id *ID) UnmarshalText(b []byte) error { return decodeHex(id[:], b, "id") }

// ParseKey and ParseID read the hex forms, as found in a URL path.
func ParseKey(s string) (k Key, err error) { return k, k.UnmarshalText([]byte(s)) }
func ParseID(s string) (id ID, err error)  { return id, id.UnmarshalText([]byte(s)) }

func decodeHex(dst, text []byte, what string) error {
	if len(text) != 2*len(dst) {
		return fmt.Errorf("%s must be %d hex digits", what, 2*len(dst))
	}
	if _, err := hex.Decode(dst, text); err != nil {
		return fmt.Errorf("%s must be %d hex digits", what, 2*len(dst))
	}
	return nil
}

// CleanName trims a display, space or room name and checks it: 1 to 32
// characters, valid UTF-8, no control characters. Names are shown to other
// members, so nothing in them may move a cursor or hide text.
func CleanName(name string) (string, error) {
	name = strings.TrimSpace(name)
	if !utf8.ValidString(name) || name == "" || utf8.RuneCountInString(name) > 32 {
		return "", ErrBadName
	}
	for _, r := range name {
		if unicode.IsControl(r) || unicode.Is(unicode.Cf, r) {
			return "", ErrBadName
		}
	}
	return name, nil
}

// Store is the open database. It is safe for concurrent use.
type Store struct {
	db     *sql.DB
	ledger *ledger.Ledger
	now    func() time.Time
}

// Open opens or creates the database at path, brings its schema up to
// date, and records deletions in l. now is the clock; pass time.Now outside
// tests.
func Open(path string, l *ledger.Ledger, now func() time.Time) (*Store, error) {
	if strings.ContainsAny(path, "?#") {
		return nil, fmt.Errorf("database path %q: ? and # are not allowed", path)
	}
	// Personal data: readable by the server's user only. SQLite gives its
	// -wal and -shm files the database's permissions.
	f, err := os.OpenFile(path, os.O_RDWR|os.O_CREATE, 0o600)
	if err != nil {
		return nil, err
	}
	f.Close()
	// Foreign keys are off by default in SQLite; WAL lets readers work
	// while a write is in progress.
	dsn := "file:" + path + "?_pragma=foreign_keys(1)&_pragma=journal_mode(WAL)&_pragma=busy_timeout(5000)"
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, err
	}
	// One connection: writes are serialised anyway, and a control plane
	// for a few teams never needs more. It also means no two transactions
	// ever wait on each other's locks.
	db.SetMaxOpenConns(1)
	if err := migrate(db); err != nil {
		db.Close()
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	return &Store{db: db, ledger: l, now: now}, nil
}

// Close closes the database.
func (s *Store) Close() error { return s.db.Close() }

// tx runs f in a transaction, committed if f returns nil.
func (s *Store) tx(ctx context.Context, f func(*sql.Tx) error) error {
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	if err := f(tx); err != nil {
		tx.Rollback()
		return err
	}
	return tx.Commit()
}

// unix and fromUnix convert between the database's Unix seconds and Go.
func (s *Store) unix() int64           { return s.now().Unix() }
func fromUnix(seconds int64) time.Time { return time.Unix(seconds, 0).UTC() }
