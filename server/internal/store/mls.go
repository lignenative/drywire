package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"time"
)

// The MLS delivery service (ADR 0052): each space's group lives on its
// members' devices; the server only keeps, in order, the bytes they hand
// each other, and reads none of them. A device is named by its own MLS
// public key: a person's devices share their account key (ADR 0051), but
// each is a leaf of its own in the group.

// Device is a device's MLS public key.
type Device [32]byte

// MaxMLSMessage bounds one key package, commit or welcome.
const MaxMLSMessage = 16 * 1024

// maxCommitsListed bounds one answer of Commits: a device further behind
// asks again from where it got to.
const maxCommitsListed = 64

// How long the delivery service keeps what it holds (spec section 9): a
// key package or a welcome nobody took within a month is stale, since the
// device posts a new one when it comes back; a device away longer than the
// commits are kept joins again as a new leaf.
const (
	mlsWaitingKept = 30 * 24 * time.Hour
	mlsCommitsKept = 90 * 24 * time.Hour
)

// ErrMLSTooLarge is a message over MaxMLSMessage.
var ErrMLSTooLarge = errors.New("an MLS message holds at most 16 KiB")

// ErrDeviceTaken is a key package for a device another identity posted
// for.
var ErrDeviceTaken = errors.New("this device belongs to another member")

// StaleEpochError refuses a commit made in an epoch that is no longer the
// group's: another member committed first. The device catches up and
// tries again.
type StaleEpochError struct{ Current uint64 }

func (e StaleEpochError) Error() string {
	return fmt.Sprintf("the group is at epoch %d: catch up, then commit again", e.Current)
}

// PendingDevice is a device that waits to be added to a space's group.
type PendingDevice struct {
	Device     Device
	PublicKey  Key // its account's key, as the members list shows it
	KeyPackage []byte
}

// Welcome is what lets one device join the group.
type Welcome struct {
	Device Device
	Body   []byte
}

// CommitRow is one commit, in the order the group took them.
type CommitRow struct {
	Epoch uint64 // the epoch it started
	Body  []byte
}

// PutKeyPackage records that a member's device waits to join the space's
// group, replacing what that device posted before.
func (s *Store) PutKeyPackage(ctx context.Context, space ID, identityID int64, device Device, body []byte) error {
	if len(body) > MaxMLSMessage {
		return ErrMLSTooLarge
	}
	return s.tx(ctx, func(tx *sql.Tx) error {
		// A device belongs to the identity that first posted for it: no
		// other member may take its place in the queue, nor its welcome.
		var owner int64
		err := tx.QueryRowContext(ctx, `
			SELECT identity_id FROM mls_key_packages WHERE space_id = ? AND device = ?
			UNION ALL
			SELECT identity_id FROM mls_welcomes WHERE space_id = ? AND device = ?
			LIMIT 1`, space[:], device[:], space[:], device[:]).Scan(&owner)
		if err == nil && owner != identityID {
			return ErrDeviceTaken
		}
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		_, err = tx.ExecContext(ctx, `
			INSERT INTO mls_key_packages (space_id, device, identity_id, body, created_at) VALUES (?, ?, ?, ?, ?)
			ON CONFLICT (space_id, device) DO UPDATE SET body = excluded.body, created_at = excluded.created_at`,
			space[:], device[:], identityID, body, s.unix())
		return err
	})
}

// KeyPackages lists the devices that wait to join the space's group,
// oldest first.
func (s *Store) KeyPackages(ctx context.Context, space ID) ([]PendingDevice, error) {
	rows, err := s.db.QueryContext(ctx, `
		SELECT k.device, i.public_key, k.body FROM mls_key_packages k
		JOIN identities i ON i.id = k.identity_id
		WHERE k.space_id = ? ORDER BY k.created_at, k.device`, space[:])
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []PendingDevice{}
	for rows.Next() {
		var p PendingDevice
		var device, key []byte
		if err := rows.Scan(&device, &key, &p.KeyPackage); err != nil {
			return nil, err
		}
		copy(p.Device[:], device)
		copy(p.PublicKey[:], key)
		out = append(out, p)
	}
	return out, rows.Err()
}

// MLSEpoch is the epoch of the space's group as the server ordered it: 0
// before the first commit.
func (s *Store) MLSEpoch(ctx context.Context, space ID) (uint64, error) {
	var epoch sql.NullInt64
	err := s.db.QueryRowContext(ctx, `SELECT max(epoch) FROM mls_commits WHERE space_id = ?`, space[:]).Scan(&epoch)
	return uint64(epoch.Int64), err
}

// Commit appends a commit made in epoch `from`, which must be the group's
// current one, and the welcomes it makes; the devices welcomed stop
// waiting. It returns the epoch the commit started, or StaleEpochError.
func (s *Store) Commit(ctx context.Context, space ID, from uint64, body []byte, welcomes []Welcome) (uint64, error) {
	if len(body) > MaxMLSMessage {
		return 0, ErrMLSTooLarge
	}
	for _, w := range welcomes {
		if len(w.Body) > MaxMLSMessage {
			return 0, ErrMLSTooLarge
		}
	}
	var next uint64
	err := s.tx(ctx, func(tx *sql.Tx) error {
		var current sql.NullInt64
		if err := tx.QueryRowContext(ctx, `SELECT max(epoch) FROM mls_commits WHERE space_id = ?`, space[:]).Scan(&current); err != nil {
			return err
		}
		if uint64(current.Int64) != from {
			return StaleEpochError{Current: uint64(current.Int64)}
		}
		next = from + 1
		now := s.unix()
		if _, err := tx.ExecContext(ctx, `INSERT INTO mls_commits (space_id, epoch, body, created_at) VALUES (?, ?, ?, ?)`,
			space[:], next, body, now); err != nil {
			return err
		}
		for _, w := range welcomes {
			// Only a device that asked to join can be welcomed: its key
			// package says whose it is.
			var identityID int64
			err := tx.QueryRowContext(ctx, `SELECT identity_id FROM mls_key_packages WHERE space_id = ? AND device = ?`,
				space[:], w.Device[:]).Scan(&identityID)
			if errors.Is(err, sql.ErrNoRows) {
				return ErrNotFound
			}
			if err != nil {
				return err
			}
			if _, err := tx.ExecContext(ctx, `
				INSERT INTO mls_welcomes (space_id, device, identity_id, epoch, body, created_at) VALUES (?, ?, ?, ?, ?, ?)
				ON CONFLICT (space_id, device) DO UPDATE SET epoch = excluded.epoch,
					body = excluded.body, created_at = excluded.created_at
				WHERE mls_welcomes.identity_id = excluded.identity_id`,
				space[:], w.Device[:], identityID, next, w.Body, now); err != nil {
				return err
			}
			if _, err := tx.ExecContext(ctx, `DELETE FROM mls_key_packages WHERE space_id = ? AND device = ?`,
				space[:], w.Device[:]); err != nil {
				return err
			}
		}
		return nil
	})
	return next, err
}

// Commits lists the commits that started epochs after `after`, in order,
// at most maxCommitsListed of them.
func (s *Store) Commits(ctx context.Context, space ID, after uint64) ([]CommitRow, error) {
	rows, err := s.db.QueryContext(ctx, `
		SELECT epoch, body FROM mls_commits WHERE space_id = ? AND epoch > ? ORDER BY epoch LIMIT ?`,
		space[:], after, maxCommitsListed)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []CommitRow{}
	for rows.Next() {
		var c CommitRow
		if err := rows.Scan(&c.Epoch, &c.Body); err != nil {
			return nil, err
		}
		out = append(out, c)
	}
	return out, rows.Err()
}

// WelcomeFor returns the welcome a device was given, and the epoch it
// joins, if the identity asking is the device's.
func (s *Store) WelcomeFor(ctx context.Context, space ID, identityID int64, device Device) (uint64, []byte, error) {
	var epoch uint64
	var body []byte
	err := s.db.QueryRowContext(ctx, `
		SELECT epoch, body FROM mls_welcomes WHERE space_id = ? AND device = ? AND identity_id = ?`,
		space[:], device[:], identityID).Scan(&epoch, &body)
	if errors.Is(err, sql.ErrNoRows) {
		return 0, nil, ErrNotFound
	}
	return epoch, body, err
}

// purgeExpiredMLS deletes the key packages and welcomes nobody took within
// mlsWaitingKept, and the commits older than mlsCommitsKept except each
// space's last, which says the group's epoch.
func (s *Store) purgeExpiredMLS(ctx context.Context) error {
	now := s.now()
	waiting := now.Add(-mlsWaitingKept).Unix()
	for _, table := range []string{"mls_key_packages", "mls_welcomes"} {
		if _, err := s.db.ExecContext(ctx, `DELETE FROM `+table+` WHERE created_at < ?`, waiting); err != nil {
			return err
		}
	}
	_, err := s.db.ExecContext(ctx, `
		DELETE FROM mls_commits WHERE created_at < ?
		AND epoch < (SELECT max(epoch) FROM mls_commits c WHERE c.space_id = mls_commits.space_id)`,
		now.Add(-mlsCommitsKept).Unix())
	return err
}
