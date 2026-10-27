package store

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"errors"
	"time"
)

// CodeSize is the size of an invitation code: 128 random bits, too many to
// guess.
const CodeSize = 16

// Invitation is an invitation code as admins see it. The code itself is
// shown once, when it is created, and never stored.
type Invitation struct {
	ID        ID        `json:"id"`
	SpaceID   ID        `json:"space_id"`
	CreatedAt time.Time `json:"created_at"`
	ExpiresAt time.Time `json:"expires_at"`
	MaxUses   int       `json:"max_uses"`
	Uses      int       `json:"uses"`
}

func codeHash(code [CodeSize]byte) []byte {
	sum := sha256.Sum256(code[:])
	return sum[:]
}

// CreateInvitation stores the hash of a new code for the space.
func (s *Store) CreateInvitation(ctx context.Context, space ID, createdBy int64, code [CodeSize]byte, lifetime time.Duration, maxUses int) (Invitation, error) {
	now := s.unix()
	inv := Invitation{
		ID:        newID(),
		SpaceID:   space,
		CreatedAt: fromUnix(now),
		ExpiresAt: fromUnix(now + int64(lifetime/time.Second)),
		MaxUses:   maxUses,
	}
	_, err := s.db.ExecContext(ctx, `
		INSERT INTO invitations (id, code_hash, space_id, created_by, created_at, expires_at, max_uses)
		VALUES (?, ?, ?, ?, ?, ?, ?)`,
		inv.ID[:], codeHash(code), space[:], createdBy, now, inv.ExpiresAt.Unix(), maxUses)
	return inv, err
}

// Invitations returns the space's invitations that can still be used.
func (s *Store) Invitations(ctx context.Context, space ID) ([]Invitation, error) {
	rows, err := s.db.QueryContext(ctx, `
		SELECT id, created_at, expires_at, max_uses, uses FROM invitations
		WHERE space_id = ? AND expires_at > ? AND uses < max_uses ORDER BY created_at`,
		space[:], s.unix())
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []Invitation{}
	for rows.Next() {
		inv := Invitation{SpaceID: space}
		var id []byte
		var created, expires int64
		if err := rows.Scan(&id, &created, &expires, &inv.MaxUses, &inv.Uses); err != nil {
			return nil, err
		}
		copy(inv.ID[:], id)
		inv.CreatedAt, inv.ExpiresAt = fromUnix(created), fromUnix(expires)
		out = append(out, inv)
	}
	return out, rows.Err()
}

// RevokeInvitation deletes an invitation of the space.
func (s *Store) RevokeInvitation(ctx context.Context, space, id ID) error {
	res, err := s.db.ExecContext(ctx, `DELETE FROM invitations WHERE id = ? AND space_id = ?`, id[:], space[:])
	if err != nil {
		return err
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return ErrNotFound
	}
	return nil
}

// Redeem uses an invitation code: the holder of key joins its space as a
// member, created with name if the identity is new. Someone who is already
// a member gets the space back without using up the code.
//
// An unknown, expired and used-up code all give ErrInvalidCode, so the
// answer says nothing about which codes exist.
func (s *Store) Redeem(ctx context.Context, code [CodeSize]byte, key Key, name string) (Space, error) {
	var space Space
	err := s.tx(ctx, func(tx *sql.Tx) error {
		var invID, spaceID []byte
		var expires int64
		var maxUses, uses int
		err := tx.QueryRowContext(ctx,
			`SELECT id, space_id, expires_at, max_uses, uses FROM invitations WHERE code_hash = ?`,
			codeHash(code)).Scan(&invID, &spaceID, &expires, &maxUses, &uses)
		if errors.Is(err, sql.ErrNoRows) {
			return ErrInvalidCode
		}
		if err != nil {
			return err
		}
		if expires <= s.unix() || uses >= maxUses {
			return ErrInvalidCode
		}
		copy(space.ID[:], spaceID)

		var banned bool
		if err := tx.QueryRowContext(ctx,
			`SELECT EXISTS (SELECT 1 FROM bans WHERE space_id = ? AND public_key = ?)`,
			spaceID, key[:]).Scan(&banned); err != nil {
			return err
		}
		if banned {
			return ErrBanned
		}

		var created int64
		if err := tx.QueryRowContext(ctx, `SELECT name, max_members, created_at FROM spaces WHERE id = ?`, spaceID).
			Scan(&space.Name, &space.MaxMembers, &created); err != nil {
			return err
		}
		space.CreatedAt = fromUnix(created)

		id, err := s.ensureIdentity(ctx, tx, key, name)
		if err != nil {
			return err
		}
		var already bool
		if err := tx.QueryRowContext(ctx,
			`SELECT EXISTS (SELECT 1 FROM members WHERE space_id = ? AND identity_id = ?)`,
			spaceID, id.ID).Scan(&already); err != nil {
			return err
		}
		if already {
			return nil
		}
		var count int
		if err := tx.QueryRowContext(ctx, `SELECT count(*) FROM members WHERE space_id = ?`, spaceID).Scan(&count); err != nil {
			return err
		}
		if count >= space.MaxMembers {
			return ErrSpaceFull
		}
		if _, err := tx.ExecContext(ctx,
			`INSERT INTO members (space_id, identity_id, role, joined_at) VALUES (?, ?, 'member', ?)`,
			spaceID, id.ID, s.unix()); err != nil {
			return err
		}
		_, err = tx.ExecContext(ctx, `UPDATE invitations SET uses = uses + 1 WHERE id = ?`, invID)
		return err
	})
	return space, err
}
