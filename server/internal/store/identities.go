package store

import (
	"context"
	"database/sql"
	"encoding/hex"
	"errors"
	"time"
)

// Identity is a person's account (ADR 0055): their e-mail address, user
// name and display name, and the key of their first device. Each device
// logs in with a key of its own (devices).
type Identity struct {
	ID         int64     `json:"-"`
	PublicKey  Key       `json:"public_key"`
	Name       string    `json:"name"`
	Email      string    `json:"-"` // never shown to others
	Username   string    `json:"username,omitempty"`
	CreatedAt  time.Time `json:"created_at"`
	LastSeenAt time.Time `json:"last_seen_at"`
}

const identityColumns = `i.id, i.public_key, i.name, coalesce(i.email, ''), coalesce(i.username, ''), i.created_at, i.last_seen_at`

func scanIdentity(row interface{ Scan(...any) error }) (Identity, error) {
	var id Identity
	var key []byte
	var created, seen int64
	err := row.Scan(&id.ID, &key, &id.Name, &id.Email, &id.Username, &created, &seen)
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

// IdentityByKey returns the identity one of whose devices has this public
// key, or ErrNotFound.
func (s *Store) IdentityByKey(ctx context.Context, key Key) (Identity, error) {
	return scanIdentity(s.db.QueryRowContext(ctx,
		`SELECT `+identityColumns+` FROM devices d JOIN identities i ON i.id = d.identity_id WHERE d.public_key = ?`, key[:]))
}

// ensureIdentity returns the identity with this key, creating it with name
// if it does not exist. An existing identity keeps its name.
func (s *Store) ensureIdentity(ctx context.Context, tx *sql.Tx, key Key, name string) (Identity, error) {
	id, err := scanIdentity(tx.QueryRowContext(ctx,
		`SELECT `+identityColumns+` FROM devices d JOIN identities i ON i.id = d.identity_id WHERE d.public_key = ?`, key[:]))
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
	if _, err := tx.ExecContext(ctx,
		`INSERT INTO devices (public_key, identity_id, created_at, last_seen_at) VALUES (?, ?, ?, ?)`,
		key[:], rowID, now, now); err != nil {
		return id, err
	}
	return Identity{ID: rowID, PublicKey: key, Name: name, CreatedAt: fromUnix(now), LastSeenAt: fromUnix(now)}, nil
}

// Touch records that the identity logged in now. Identities not seen for
// a year are purged (PurgeInactive).
func (s *Store) Touch(ctx context.Context, key Key) error {
	now := s.unix()
	if _, err := s.db.ExecContext(ctx, `UPDATE devices SET last_seen_at = ? WHERE public_key = ?`, now, key[:]); err != nil {
		return err
	}
	_, err := s.db.ExecContext(ctx,
		`UPDATE identities SET last_seen_at = ? WHERE id = (SELECT identity_id FROM devices WHERE public_key = ?)`, now, key[:])
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
	Email       string             `json:"email,omitempty"`
	Devices     []AccountDevice    `json:"devices"`
	Received    []UserInvitation   `json:"invitations_received"`
	Memberships []ExportMembership `json:"memberships"`
	Invitations []ExportInvitation `json:"invitations_created"`
	Vaults      []ExportVault      `json:"vaults"`
	MLSDevices  []ExportMLSDevice  `json:"mls_devices"`
	Note        string             `json:"note"`
}

// ExportMembership is one space the identity belongs to.
type ExportMembership struct {
	SpaceID   ID        `json:"space_id"`
	SpaceName string    `json:"space_name"`
	Role      Role      `json:"role"`
	JoinedAt  time.Time `json:"joined_at"`
}

// ExportVault is one vault of the identity, as the server sees it: sealed
// bytes it cannot open, so only their size and dates.
type ExportVault struct {
	Kind      VaultKind  `json:"kind"`
	Size      int        `json:"size"`
	CreatedAt time.Time  `json:"created_at"`
	ExpiresAt *time.Time `json:"expires_at,omitempty"`
}

// ExportMLSDevice is one of the identity's devices that waits to join a
// space's MLS group, or was given what joins it (ADR 0052). Its key
// package and welcome are the group's messages, encrypted to the device
// or meant for the members' apps, so only their size is shown.
type ExportMLSDevice struct {
	SpaceID   ID        `json:"space_id"`
	Device    string    `json:"device"`
	State     string    `json:"state"` // "waiting" or "welcomed"
	Size      int       `json:"size"`
	CreatedAt time.Time `json:"created_at"`
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
			`SELECT `+identityColumns+` FROM identities i WHERE i.id = ?`, identityID))
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
		if err := rows.Err(); err != nil {
			return err
		}

		vaults, err := tx.QueryContext(ctx, `
			SELECT kind, length(sealed), created_at, expires_at
			FROM vaults WHERE identity_id = ? ORDER BY created_at`, identityID)
		if err != nil {
			return err
		}
		defer vaults.Close()
		e.Vaults = []ExportVault{}
		for vaults.Next() {
			var v ExportVault
			var created int64
			var expires sql.NullInt64
			if err := vaults.Scan(&v.Kind, &v.Size, &created, &expires); err != nil {
				return err
			}
			v.CreatedAt = fromUnix(created)
			if expires.Valid {
				t := fromUnix(expires.Int64)
				v.ExpiresAt = &t
			}
			e.Vaults = append(e.Vaults, v)
		}
		if err := vaults.Err(); err != nil {
			return err
		}

		devices, err := tx.QueryContext(ctx, `
			SELECT space_id, device, 'waiting', length(body), created_at FROM mls_key_packages WHERE identity_id = ?
			UNION ALL
			SELECT w.space_id, w.device, 'welcomed', coalesce(length(c.welcome), 0), w.created_at FROM mls_welcomes w
			LEFT JOIN mls_commits c ON c.space_id = w.space_id AND c.epoch = w.epoch WHERE w.identity_id = ?
			ORDER BY 5`, identityID, identityID)
		if err != nil {
			return err
		}
		defer devices.Close()
		e.MLSDevices = []ExportMLSDevice{}
		for devices.Next() {
			var d ExportMLSDevice
			var space, device []byte
			var created int64
			if err := devices.Scan(&space, &device, &d.State, &d.Size, &created); err != nil {
				return err
			}
			copy(d.SpaceID[:], space)
			d.Device = hex.EncodeToString(device)
			d.CreatedAt = fromUnix(created)
			e.MLSDevices = append(e.MLSDevices, d)
		}
		return devices.Err()
	})
	if err == nil {
		e.Email = e.Identity.Email
		if e.Devices, err = s.Devices(ctx, identityID); err == nil {
			e.Received, err = s.UserInvitations(ctx, identityID)
		}
	}
	e.Note = "Everything this Drywire server stores about this identity. " +
		"Presence (who is in which room) is kept in memory only and never stored; " +
		"audio is end-to-end encrypted and never stored. " +
		"Vaults are sealed on your device with a code this server never sees, so only their size is shown."
	return e, err
}
