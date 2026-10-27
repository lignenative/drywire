// Package ledger is the deletion ledger: an append-only file that records
// every deletion of personal data, so that a deletion survives the restore
// of an older backup (docs/spec.md, section 9).
//
// The ledger lives next to the database but is never part of a backup. At
// every start the server replays it against the database: whatever a
// restored backup brought back is deleted again.
//
// An entry holds as little as possible. An identity is recorded by the
// SHA-256 of its public key, never the key itself, so the ledger cannot say
// who was deleted, only recognise them if a backup brings them back.
package ledger

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"time"
)

// What an entry deletes.
const (
	// KindIdentity: an identity and everything attached to it.
	KindIdentity = "identity"
	// KindSpace: a space, its rooms, members, bans and invitations.
	KindSpace = "space"
	// KindMembership: one identity's membership of one space.
	KindMembership = "membership"
	// KindInvitation: a revoked invitation, which must not work again.
	KindInvitation = "invitation"
)

// Retention is how long entries are kept. A backup is kept six months at
// most (spec section 9), so an older entry can no longer meet anything to
// delete again; a month of margin covers a late restore.
const Retention = 213 * 24 * time.Hour

// Entry is one deletion. Fields not used by its kind are empty.
type Entry struct {
	// At is when the deletion happened. Replay only deletes rows created
	// before it, so a person who comes back later is not deleted again.
	At int64 `json:"at"` // Unix seconds
	// Kind is one of the Kind constants.
	Kind string `json:"kind"`
	// Identity is the hex SHA-256 of the public key (KindIdentity and
	// KindMembership).
	Identity string `json:"identity,omitempty"`
	// Space is the space's hex id (KindSpace and KindMembership).
	Space string `json:"space,omitempty"`
	// Invitation is the invitation's hex id (KindInvitation).
	Invitation string `json:"invitation,omitempty"`
}

// KeyHash is how an entry refers to an identity: the hex SHA-256 of its
// public key.
func KeyHash(publicKey []byte) string {
	sum := sha256.Sum256(publicKey)
	return hex.EncodeToString(sum[:])
}

// Ledger is an open ledger file. It is safe for concurrent use.
type Ledger struct {
	mu   sync.Mutex
	path string
	file *os.File
}

// Open opens the ledger at path, creating it, readable by its owner only,
// if it does not exist.
func Open(path string) (*Ledger, error) {
	f, err := openAppend(path)
	if err != nil {
		return nil, err
	}
	return &Ledger{path: path, file: f}, nil
}

func openAppend(path string) (*os.File, error) {
	return os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
}

// Append writes one entry and waits until it is on disk: the caller deletes
// the data only after this returns, so a deletion is never lost by a crash
// in between. If the deletion then fails (its transaction rolls back), the
// entry stays, and the next replay carries out the deletion that was asked
// for: the safe direction.
func (l *Ledger) Append(e Entry) error {
	line, err := json.Marshal(e)
	if err != nil {
		return err
	}
	l.mu.Lock()
	defer l.mu.Unlock()
	if _, err := l.file.Write(append(line, '\n')); err != nil {
		return fmt.Errorf("deletion ledger: %w", err)
	}
	if err := l.file.Sync(); err != nil {
		return fmt.Errorf("deletion ledger: %w", err)
	}
	return nil
}

// Entries returns every entry, oldest first.
func (l *Ledger) Entries() ([]Entry, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	return l.read()
}

func (l *Ledger) read() ([]Entry, error) {
	raw, err := os.ReadFile(l.path)
	if err != nil {
		return nil, err
	}
	var entries []Entry
	lines := bytes.Split(raw, []byte("\n"))
	for n, line := range lines {
		if len(bytes.TrimSpace(line)) == 0 {
			continue
		}
		var e Entry
		if err := json.Unmarshal(line, &e); err != nil {
			// A last line without its newline was cut short by a crash
			// during Append, before its deletion happened: skip it.
			// Damage anywhere else is worth stopping for.
			if n == len(lines)-1 {
				break
			}
			return nil, fmt.Errorf("%s line %d: %w", l.path, n+1, err)
		}
		entries = append(entries, e)
	}
	return entries, nil
}

// Prune drops the entries older than before. The file is rewritten beside
// the old one and renamed over it, so a crash leaves either the old or the
// new ledger, never half of one.
func (l *Ledger) Prune(before time.Time) (removed int, err error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	entries, err := l.read()
	if err != nil {
		return 0, err
	}
	var kept bytes.Buffer
	for _, e := range entries {
		if e.At < before.Unix() {
			removed++
			continue
		}
		line, _ := json.Marshal(e)
		kept.Write(append(line, '\n'))
	}
	if removed == 0 {
		return 0, nil
	}

	tmp, err := os.CreateTemp(filepath.Dir(l.path), ".ledger-*")
	if err != nil {
		return 0, err
	}
	defer os.Remove(tmp.Name()) // a no-op once renamed
	if err := tmp.Chmod(0o600); err != nil {
		tmp.Close()
		return 0, err
	}
	if _, err := tmp.Write(kept.Bytes()); err != nil {
		tmp.Close()
		return 0, err
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return 0, err
	}
	if err := tmp.Close(); err != nil {
		return 0, err
	}
	if err := os.Rename(tmp.Name(), l.path); err != nil {
		return 0, err
	}
	// The open handle still points to the old file: reopen.
	f, err := openAppend(l.path)
	if err != nil {
		return 0, err
	}
	l.file.Close()
	l.file = f
	return removed, nil
}

// Path is where the ledger is.
func (l *Ledger) Path() string { return l.path }

// Close closes the file.
func (l *Ledger) Close() error {
	l.mu.Lock()
	defer l.mu.Unlock()
	return l.file.Close()
}
