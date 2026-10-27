package store

import (
	"context"
	"database/sql"
	"encoding/hex"
	"errors"
	"time"

	"github.com/lignenative/drywire/server/internal/ledger"
)

// InactiveAfter is how long an identity may go without logging in before
// it is purged (spec section 9).
const InactiveAfter = 365 * 24 * time.Hour

// Deleted says what a deletion removed, so the caller can end the live
// sessions and presence that went with it.
type Deleted struct {
	// Spaces deleted because their owner left nobody to hand them to.
	Spaces []ID
	// Spaces the identity was taken out of, the deleted ones included.
	Memberships []ID
}

// DeleteIdentity deletes an identity and its memberships. Each space it
// owned goes to its oldest admin, or failing that its oldest member; a
// space with no one else in it is deleted. The deletion is written to the
// ledger before the database changes.
func (s *Store) DeleteIdentity(ctx context.Context, identityID int64) (Deleted, error) {
	var d Deleted
	err := s.tx(ctx, func(tx *sql.Tx) error {
		var err error
		d, err = s.deleteIdentity(ctx, tx, identityID, true)
		return err
	})
	return d, err
}

func (s *Store) deleteIdentity(ctx context.Context, tx *sql.Tx, identityID int64, record bool) (Deleted, error) {
	var d Deleted
	var key []byte
	err := tx.QueryRowContext(ctx, `SELECT public_key FROM identities WHERE id = ?`, identityID).Scan(&key)
	if errors.Is(err, sql.ErrNoRows) {
		return d, ErrNotFound
	}
	if err != nil {
		return d, err
	}
	if record {
		if err := s.ledger.Append(ledger.Entry{At: s.unix(), Kind: ledger.KindIdentity, Identity: ledger.KeyHash(key)}); err != nil {
			return d, err
		}
	}
	spaces, err := spaceIDs(ctx, tx, `SELECT space_id FROM members WHERE identity_id = ?`, identityID)
	if err != nil {
		return d, err
	}
	for _, space := range spaces {
		gone, err := s.removeMemberRow(ctx, tx, space, identityID, record)
		if err != nil {
			return d, err
		}
		d.Memberships = append(d.Memberships, space)
		if gone {
			d.Spaces = append(d.Spaces, space)
		}
	}
	_, err = tx.ExecContext(ctx, `DELETE FROM identities WHERE id = ?`, identityID)
	return d, err
}

// removeMember records the removal in the ledger, then takes the identity
// out of the space.
func (s *Store) removeMember(ctx context.Context, tx *sql.Tx, space ID, identityID int64, record bool) error {
	if record {
		var key []byte
		err := tx.QueryRowContext(ctx, `SELECT public_key FROM identities WHERE id = ?`, identityID).Scan(&key)
		if errors.Is(err, sql.ErrNoRows) {
			return ErrNotFound
		}
		if err != nil {
			return err
		}
		if err := s.ledger.Append(ledger.Entry{
			At: s.unix(), Kind: ledger.KindMembership, Identity: ledger.KeyHash(key), Space: space.String(),
		}); err != nil {
			return err
		}
	}
	_, err := s.removeMemberRow(ctx, tx, space, identityID, record)
	return err
}

// removeMemberRow deletes one membership, and the invitations the member
// created in that space: someone removed must not come back with a code
// of their own. If it was the owner's, the space goes to the oldest admin,
// else the oldest member, else it is deleted (spaceGone).
func (s *Store) removeMemberRow(ctx context.Context, tx *sql.Tx, space ID, identityID int64, record bool) (spaceGone bool, err error) {
	var role Role
	err = tx.QueryRowContext(ctx, `SELECT role FROM members WHERE space_id = ? AND identity_id = ?`,
		space[:], identityID).Scan(&role)
	if errors.Is(err, sql.ErrNoRows) {
		return false, ErrNotFound
	}
	if err != nil {
		return false, err
	}
	if _, err := tx.ExecContext(ctx, `DELETE FROM members WHERE space_id = ? AND identity_id = ?`, space[:], identityID); err != nil {
		return false, err
	}
	if _, err := tx.ExecContext(ctx, `DELETE FROM invitations WHERE space_id = ? AND created_by = ?`, space[:], identityID); err != nil {
		return false, err
	}
	if role != RoleOwner {
		return false, nil
	}
	var heir int64
	err = tx.QueryRowContext(ctx, `
		SELECT identity_id FROM members WHERE space_id = ?
		ORDER BY CASE role WHEN 'admin' THEN 0 ELSE 1 END, joined_at, identity_id LIMIT 1`,
		space[:]).Scan(&heir)
	if errors.Is(err, sql.ErrNoRows) {
		return true, s.deleteSpace(ctx, tx, space, record)
	}
	if err != nil {
		return false, err
	}
	_, err = tx.ExecContext(ctx, `UPDATE members SET role = 'owner' WHERE space_id = ? AND identity_id = ?`, space[:], heir)
	return false, err
}

func (s *Store) deleteSpace(ctx context.Context, tx *sql.Tx, space ID, record bool) error {
	if record {
		if err := s.ledger.Append(ledger.Entry{At: s.unix(), Kind: ledger.KindSpace, Space: space.String()}); err != nil {
			return err
		}
	}
	// Rooms, members, bans and invitations go with it (ON DELETE CASCADE).
	res, err := tx.ExecContext(ctx, `DELETE FROM spaces WHERE id = ?`, space[:])
	if err != nil {
		return err
	}
	if n, _ := res.RowsAffected(); n == 0 && record {
		return ErrNotFound
	}
	return nil
}

func spaceIDs(ctx context.Context, tx *sql.Tx, query string, args ...any) ([]ID, error) {
	rows, err := tx.QueryContext(ctx, query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []ID
	for rows.Next() {
		var raw []byte
		if err := rows.Scan(&raw); err != nil {
			return nil, err
		}
		var id ID
		copy(id[:], raw)
		out = append(out, id)
	}
	return out, rows.Err()
}

// Purged says what PurgeInactive removed.
type Purged struct {
	Identities  int
	Invitations int
	// Deleted lists, per purged identity, what went with it.
	Deleted []Deleted
}

// PurgeInactive deletes the identities that have not logged in for
// InactiveAfter, exactly as if each had deleted itself, and the
// invitations that can no longer be used. The server runs it daily.
func (s *Store) PurgeInactive(ctx context.Context) (Purged, error) {
	var p Purged
	now := s.now()
	cutoff := now.Add(-InactiveAfter).Unix()
	rows, err := s.db.QueryContext(ctx, `SELECT id FROM identities WHERE last_seen_at < ?`, cutoff)
	if err != nil {
		return p, err
	}
	var ids []int64
	for rows.Next() {
		var id int64
		if err := rows.Scan(&id); err != nil {
			rows.Close()
			return p, err
		}
		ids = append(ids, id)
	}
	rows.Close()
	if err := rows.Err(); err != nil {
		return p, err
	}
	for _, id := range ids {
		d, err := s.DeleteIdentity(ctx, id)
		if errors.Is(err, ErrNotFound) {
			continue // deleted meanwhile
		}
		if err != nil {
			return p, err
		}
		p.Identities++
		p.Deleted = append(p.Deleted, d)
	}
	res, err := s.db.ExecContext(ctx, `DELETE FROM invitations WHERE expires_at <= ? OR uses >= max_uses`, now.Unix())
	if err != nil {
		return p, err
	}
	n, _ := res.RowsAffected()
	p.Invitations = int(n)
	return p, nil
}

// Replay applies the ledger's deletions to the database again. The server
// runs it at every start, so whatever the restore of an older backup
// brought back is deleted again. It writes nothing to the ledger.
//
// Each deletion only applies to rows created before it: someone who
// deleted their identity and later joined again with the same key keeps
// the new identity. Times are whole seconds, so a row created in the same
// second as the deletion counts as newer: a retry right after a leave must
// not be undone at the next start.
func (s *Store) Replay(ctx context.Context, entries []ledger.Entry) (applied int, err error) {
	err = s.tx(ctx, func(tx *sql.Tx) error {
		// Identities are recorded by the hash of their key: hash every
		// key once to find them.
		byHash := map[string]struct {
			id      int64
			created int64
		}{}
		rows, err := tx.QueryContext(ctx, `SELECT id, public_key, created_at FROM identities`)
		if err != nil {
			return err
		}
		for rows.Next() {
			var id, created int64
			var key []byte
			if err := rows.Scan(&id, &key, &created); err != nil {
				rows.Close()
				return err
			}
			byHash[ledger.KeyHash(key)] = struct{ id, created int64 }{id, created}
		}
		rows.Close()
		if err := rows.Err(); err != nil {
			return err
		}

		for _, e := range entries {
			switch e.Kind {
			case ledger.KindSpace:
				space, ok := idFromHex(e.Space)
				if !ok {
					continue
				}
				res, err := tx.ExecContext(ctx, `DELETE FROM spaces WHERE id = ? AND created_at < ?`, space[:], e.At)
				if err != nil {
					return err
				}
				if n, _ := res.RowsAffected(); n > 0 {
					applied++
				}
			case ledger.KindIdentity:
				who, ok := byHash[e.Identity]
				if !ok || who.created >= e.At {
					continue
				}
				if _, err := s.deleteIdentity(ctx, tx, who.id, false); errors.Is(err, ErrNotFound) {
					continue
				} else if err != nil {
					return err
				}
				delete(byHash, e.Identity)
				applied++
			case ledger.KindMembership:
				who, ok := byHash[e.Identity]
				space, okSpace := idFromHex(e.Space)
				if !ok || !okSpace {
					continue
				}
				var joined int64
				err := tx.QueryRowContext(ctx, `SELECT joined_at FROM members WHERE space_id = ? AND identity_id = ?`,
					space[:], who.id).Scan(&joined)
				if errors.Is(err, sql.ErrNoRows) || (err == nil && joined >= e.At) {
					continue
				}
				if err != nil {
					return err
				}
				if _, err := s.removeMemberRow(ctx, tx, space, who.id, false); err != nil {
					return err
				}
				applied++
			case ledger.KindInvitation:
				id, ok := idFromHex(e.Invitation)
				if !ok {
					continue
				}
				res, err := tx.ExecContext(ctx, `DELETE FROM invitations WHERE id = ? AND created_at < ?`, id[:], e.At)
				if err != nil {
					return err
				}
				if n, _ := res.RowsAffected(); n > 0 {
					applied++
				}
			}
		}
		return nil
	})
	return applied, err
}

func idFromHex(s string) (ID, bool) {
	var id ID
	b, err := hex.DecodeString(s)
	if err != nil || len(b) != len(id) {
		return id, false
	}
	copy(id[:], b)
	return id, true
}
