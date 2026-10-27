package store

import (
	"context"
	"database/sql"
	"errors"
	"time"
)

// Identity is one device's key pair as the server knows it: the public key
// and the name its owner chose.
type Identity struct {
	ID         int64     `json:"-"`
	PublicKey  Key       `json:"public_key"`
	Name       string    `json:"name"`
	CreatedAt  time.Time `json:"created_at"`
	LastSeenAt time.Time `json:"last_seen_at"`
}

const identityColumns = `id, public_key, name, created_at, last_seen_at`

func scanIdentity(row interface{ Scan(...any) error }) (Identity, error) {
	var id Identity
	var key []byte
	var created, seen int64
	err := row.Scan(&id.ID, &key, &id.Name, &created, &seen)
	if errors.Is(err, sql.ErrNoRows) {
		return id, ErrNotFound
	}
	if err != nil {
		return id, err
	}
	copy(id.PublicKey[:], key)
	id.CreatedAt, id.LastSeenAt = fromUnix(created), fromUnix(seen)
	return id, nil
}

// IdentityByKey returns the identity with this public key, or ErrNotFound.
func (s *Store) IdentityByKey(ctx context.Context, key Key) (Identity, error) {
	return scanIdentity(s.db.QueryRowContext(ctx,
		`SELECT `+identityColumns+` FROM identities WHERE public_key = ?`, key[:]))
}

// ensureIdentity returns the identity with this key, creating it with name
// if it does not exist. An existing identity keeps its name.
func (s *Store) ensureIdentity(ctx context.Context, tx *sql.Tx, key Key, name string) (Identity, error) {
	id, err := scanIdentity(tx.QueryRowContext(ctx,
		`SELECT `+identityColumns+` FROM identities WHERE public_key = ?`, key[:]))
	if !errors.Is(err, ErrNotFound) {
		return id, err
	}
	if name, err = CleanName(name); err != nil {
		return id, err
	}
	now := s.unix()
	res, err := tx.ExecContext(ctx,
		`INSERT INTO identities (public_key, name, created_at, last_seen_at) VALUES (?, ?, ?, ?)`,
		key[:], name, now, now)
	if err != nil {
		return id, err
	}
	rowID, _ := res.LastInsertId()
	return Identity{ID: rowID, PublicKey: key, Name: name, CreatedAt: fromUnix(now), LastSeenAt: fromUnix(now)}, nil
}

// Touch records that the identity logged in now. Identities not seen for
// a year are purged (PurgeInactive).
func (s *Store) Touch(ctx context.Context, key Key) error {
	_, err := s.db.ExecContext(ctx, `UPDATE identities SET last_seen_at = ? WHERE public_key = ?`, s.unix(), key[:])
	return err
}

// Rename changes an identity's display name.
func (s *Store) Rename(ctx context.Context, identityID int64, name string) (string, error) {
	name, err := CleanName(name)
	if err != nil {
		return "", err
	}
	res, err := s.db.ExecContext(ctx, `UPDATE identities SET name = ? WHERE id = ?`, name, identityID)
	if err != nil {
		return "", err
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return "", ErrNotFound
	}
	return name, nil
}

// Export is everything the server holds about one identity, as the right
// of access and to data portability ask (GDPR articles 15 and 20). Live
// presence is not stored and so not part of it.
type Export struct {
	Identity    Identity           `json:"identity"`
	Memberships []ExportMembership `json:"memberships"`
	Invitations []ExportInvitation `json:"invitations_created"`
	Note        string             `json:"note"`
}

// ExportMembership is one space the identity belongs to.
type ExportMembership struct {
	SpaceID   ID        `json:"space_id"`
	SpaceName string    `json:"space_name"`
	Role      Role      `json:"role"`
	JoinedAt  time.Time `json:"joined_at"`
}

// ExportInvitation is one invitation the identity created.
type ExportInvitation struct {
	SpaceID   ID        `json:"space_id"`
	CreatedAt time.Time `json:"created_at"`
	ExpiresAt time.Time `json:"expires_at"`
	MaxUses   int       `json:"max_uses"`
	Uses      int       `json:"uses"`
}

// Export returns the identity's data.
func (s *Store) Export(ctx context.Context, identityID int64) (Export, error) {
	var e Export
	err := s.tx(ctx, func(tx *sql.Tx) error {
		var err error
		e.Identity, err = scanIdentity(tx.QueryRowContext(ctx,
			`SELECT `+identityColumns+` FROM identities WHERE id = ?`, identityID))
		if err != nil {
			return err
		}
		rows, err := tx.QueryContext(ctx, `
			SELECT s.id, s.name, m.role, m.joined_at
			FROM members m JOIN spaces s ON s.id = m.space_id
			WHERE m.identity_id = ? ORDER BY m.joined_at, s.id`, identityID)
		if err != nil {
			return err
		}
		e.Memberships = []ExportMembership{}
		for rows.Next() {
			var m ExportMembership
			var id []byte
			var joined int64
			if err := rows.Scan(&id, &m.SpaceName, &m.Role, &joined); err != nil {
				rows.Close()
				return err
			}
			copy(m.SpaceID[:], id)
			m.JoinedAt = fromUnix(joined)
			e.Memberships = append(e.Memberships, m)
		}
		rows.Close()
		if err := rows.Err(); err != nil {
			return err
		}

		rows, err = tx.QueryContext(ctx, `
			SELECT space_id, created_at, expires_at, max_uses, uses
			FROM invitations WHERE created_by = ? ORDER BY created_at`, identityID)
		if err != nil {
			return err
		}
		defer rows.Close()
		e.Invitations = []ExportInvitation{}
		for rows.Next() {
			var inv ExportInvitation
			var space []byte
			var created, expires int64
			if err := rows.Scan(&space, &created, &expires, &inv.MaxUses, &inv.Uses); err != nil {
				return err
			}
			copy(inv.SpaceID[:], space)
			inv.CreatedAt, inv.ExpiresAt = fromUnix(created), fromUnix(expires)
			e.Invitations = append(e.Invitations, inv)
		}
		return rows.Err()
	})
	e.Note = "Everything this Drywire server stores about this identity. " +
		"Presence (who is in which room) is kept in memory only and never stored; " +
		"audio is end-to-end encrypted and never stored."
	return e, err
}
