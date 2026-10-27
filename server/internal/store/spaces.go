package store

import (
	"context"
	"crypto/rand"
	"database/sql"
	"errors"
	"strings"
	"time"
)

// Space is a group of people, like a community server, with its rooms.
type Space struct {
	ID         ID        `json:"id"`
	Name       string    `json:"name"`
	MaxMembers int       `json:"max_members"`
	CreatedAt  time.Time `json:"created_at"`
}

// Room is a voice room of a space.
type Room struct {
	ID      ID     `json:"id"`
	SpaceID ID     `json:"space_id"`
	Name    string `json:"name"`
}

// Member is one member of a space, as the other members see them.
type Member struct {
	IdentityID int64     `json:"-"`
	PublicKey  Key       `json:"public_key"`
	Name       string    `json:"name"`
	Role       Role      `json:"role"`
	JoinedAt   time.Time `json:"joined_at"`
}

// Membership is one space as seen by one of its members.
type Membership struct {
	Space Space `json:"space"`
	Role  Role  `json:"role"`
}

func newID() ID {
	var id ID
	rand.Read(id[:]) // never fails (crypto/rand documentation)
	return id
}

func isUniqueViolation(err error) bool {
	return err != nil && strings.Contains(err.Error(), "UNIQUE constraint failed")
}

// CreateSpace creates a space with one room, owned by the identity with
// this key, which is created with ownerName if it is new.
func (s *Store) CreateSpace(ctx context.Context, owner Key, ownerName, spaceName, roomName string, maxMembers int) (Space, Room, error) {
	var space Space
	var room Room
	spaceName, err := CleanName(spaceName)
	if err != nil {
		return space, room, err
	}
	if roomName, err = CleanName(roomName); err != nil {
		return space, room, err
	}
	err = s.tx(ctx, func(tx *sql.Tx) error {
		id, err := s.ensureIdentity(ctx, tx, owner, ownerName)
		if err != nil {
			return err
		}
		now := s.unix()
		space = Space{ID: newID(), Name: spaceName, MaxMembers: maxMembers, CreatedAt: fromUnix(now)}
		if _, err := tx.ExecContext(ctx,
			`INSERT INTO spaces (id, name, max_members, created_at) VALUES (?, ?, ?, ?)`,
			space.ID[:], space.Name, space.MaxMembers, now); err != nil {
			return err
		}
		if _, err := tx.ExecContext(ctx,
			`INSERT INTO members (space_id, identity_id, role, joined_at) VALUES (?, ?, 'owner', ?)`,
			space.ID[:], id.ID, now); err != nil {
			return err
		}
		room = Room{ID: newID(), SpaceID: space.ID, Name: roomName}
		_, err = tx.ExecContext(ctx,
			`INSERT INTO rooms (id, space_id, name, created_at) VALUES (?, ?, ?, ?)`,
			room.ID[:], space.ID[:], room.Name, now)
		return err
	})
	return space, room, err
}

// Space returns a space, or ErrNotFound.
func (s *Store) Space(ctx context.Context, id ID) (Space, error) {
	var sp Space
	var created int64
	err := s.db.QueryRowContext(ctx,
		`SELECT name, max_members, created_at FROM spaces WHERE id = ?`, id[:]).
		Scan(&sp.Name, &sp.MaxMembers, &created)
	if errors.Is(err, sql.ErrNoRows) {
		return sp, ErrNotFound
	}
	sp.ID, sp.CreatedAt = id, fromUnix(created)
	return sp, err
}

// Role returns the identity's role in the space, or ErrNotFound if it is
// not a member.
func (s *Store) Role(ctx context.Context, space ID, identityID int64) (Role, error) {
	var role Role
	err := s.db.QueryRowContext(ctx,
		`SELECT role FROM members WHERE space_id = ? AND identity_id = ?`, space[:], identityID).Scan(&role)
	if errors.Is(err, sql.ErrNoRows) {
		return "", ErrNotFound
	}
	return role, err
}

// Memberships returns the spaces the identity belongs to.
func (s *Store) Memberships(ctx context.Context, identityID int64) ([]Membership, error) {
	rows, err := s.db.QueryContext(ctx, `
		SELECT s.id, s.name, s.max_members, s.created_at, m.role
		FROM members m JOIN spaces s ON s.id = m.space_id
		WHERE m.identity_id = ? ORDER BY s.created_at, s.id`, identityID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []Membership{}
	for rows.Next() {
		var m Membership
		var id []byte
		var created int64
		if err := rows.Scan(&id, &m.Space.Name, &m.Space.MaxMembers, &created, &m.Role); err != nil {
			return nil, err
		}
		copy(m.Space.ID[:], id)
		m.Space.CreatedAt = fromUnix(created)
		out = append(out, m)
	}
	return out, rows.Err()
}

// Members returns the members of a space, owner first.
func (s *Store) Members(ctx context.Context, space ID) ([]Member, error) {
	rows, err := s.db.QueryContext(ctx, `
		SELECT i.id, i.public_key, i.name, m.role, m.joined_at
		FROM members m JOIN identities i ON i.id = m.identity_id
		WHERE m.space_id = ?
		ORDER BY CASE m.role WHEN 'owner' THEN 0 WHEN 'admin' THEN 1 ELSE 2 END, m.joined_at, i.id`,
		space[:])
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []Member{}
	for rows.Next() {
		var m Member
		var key []byte
		var joined int64
		if err := rows.Scan(&m.IdentityID, &key, &m.Name, &m.Role, &joined); err != nil {
			return nil, err
		}
		copy(m.PublicKey[:], key)
		m.JoinedAt = fromUnix(joined)
		out = append(out, m)
	}
	return out, rows.Err()
}

// Rooms returns the rooms of a space, by name.
func (s *Store) Rooms(ctx context.Context, space ID) ([]Room, error) {
	rows, err := s.db.QueryContext(ctx, `SELECT id, name FROM rooms WHERE space_id = ? ORDER BY name`, space[:])
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []Room{}
	for rows.Next() {
		r := Room{SpaceID: space}
		var id []byte
		if err := rows.Scan(&id, &r.Name); err != nil {
			return nil, err
		}
		copy(r.ID[:], id)
		out = append(out, r)
	}
	return out, rows.Err()
}

// Room returns a room, or ErrNotFound.
func (s *Store) Room(ctx context.Context, id ID) (Room, error) {
	r := Room{ID: id}
	var space []byte
	err := s.db.QueryRowContext(ctx, `SELECT space_id, name FROM rooms WHERE id = ?`, id[:]).Scan(&space, &r.Name)
	if errors.Is(err, sql.ErrNoRows) {
		return r, ErrNotFound
	}
	copy(r.SpaceID[:], space)
	return r, err
}

// CreateRoom adds a room to a space.
func (s *Store) CreateRoom(ctx context.Context, space ID, name string) (Room, error) {
	name, err := CleanName(name)
	if err != nil {
		return Room{}, err
	}
	r := Room{ID: newID(), SpaceID: space, Name: name}
	_, err = s.db.ExecContext(ctx, `INSERT INTO rooms (id, space_id, name, created_at) VALUES (?, ?, ?, ?)`,
		r.ID[:], space[:], name, s.unix())
	if isUniqueViolation(err) {
		return r, ErrNameTaken
	}
	return r, err
}

// DeleteRoom deletes a room.
func (s *Store) DeleteRoom(ctx context.Context, id ID) error {
	res, err := s.db.ExecContext(ctx, `DELETE FROM rooms WHERE id = ?`, id[:])
	if err != nil {
		return err
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return ErrNotFound
	}
	return nil
}

// SetRole makes a member an admin or a plain member. Ownership changes
// only through TransferOwnership.
func (s *Store) SetRole(ctx context.Context, space ID, identityID int64, role Role) error {
	if role != RoleAdmin && role != RoleMember {
		return errors.New("role must be admin or member")
	}
	res, err := s.db.ExecContext(ctx,
		`UPDATE members SET role = ? WHERE space_id = ? AND identity_id = ? AND role != 'owner'`,
		role, space[:], identityID)
	if err != nil {
		return err
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return ErrNotFound
	}
	return nil
}

// TransferOwnership makes another member the owner; the old owner becomes
// an admin.
func (s *Store) TransferOwnership(ctx context.Context, space ID, from, to int64) error {
	return s.tx(ctx, func(tx *sql.Tx) error {
		res, err := tx.ExecContext(ctx,
			`UPDATE members SET role = 'admin' WHERE space_id = ? AND identity_id = ? AND role = 'owner'`, space[:], from)
		if err != nil {
			return err
		}
		if n, _ := res.RowsAffected(); n == 0 {
			return ErrNotFound
		}
		res, err = tx.ExecContext(ctx,
			`UPDATE members SET role = 'owner' WHERE space_id = ? AND identity_id = ?`, space[:], to)
		if err != nil {
			return err
		}
		if n, _ := res.RowsAffected(); n == 0 {
			return ErrNotFound
		}
		return nil
	})
}

// RemoveMember takes an identity out of a space: it left, or was removed.
// The removal is written to the deletion ledger first.
func (s *Store) RemoveMember(ctx context.Context, space ID, identityID int64) error {
	return s.tx(ctx, func(tx *sql.Tx) error {
		return s.removeMember(ctx, tx, space, identityID, true)
	})
}

// Ban removes the holder of key from the space, if they are a member, and
// stops them from joining again.
func (s *Store) Ban(ctx context.Context, space ID, key Key) error {
	return s.tx(ctx, func(tx *sql.Tx) error {
		if _, err := tx.ExecContext(ctx,
			`INSERT OR IGNORE INTO bans (space_id, public_key, created_at) VALUES (?, ?, ?)`,
			space[:], key[:], s.unix()); err != nil {
			return err
		}
		var identityID int64
		err := tx.QueryRowContext(ctx, `
			SELECT m.identity_id FROM members m JOIN identities i ON i.id = m.identity_id
			WHERE m.space_id = ? AND i.public_key = ?`, space[:], key[:]).Scan(&identityID)
		if errors.Is(err, sql.ErrNoRows) {
			return nil
		}
		if err != nil {
			return err
		}
		return s.removeMember(ctx, tx, space, identityID, true)
	})
}

// Unban lifts a ban.
func (s *Store) Unban(ctx context.Context, space ID, key Key) error {
	res, err := s.db.ExecContext(ctx, `DELETE FROM bans WHERE space_id = ? AND public_key = ?`, space[:], key[:])
	if err != nil {
		return err
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return ErrNotFound
	}
	return nil
}

// Bans returns the banned keys of a space.
func (s *Store) Bans(ctx context.Context, space ID) ([]Key, error) {
	rows, err := s.db.QueryContext(ctx, `SELECT public_key FROM bans WHERE space_id = ? ORDER BY created_at`, space[:])
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []Key{}
	for rows.Next() {
		var raw []byte
		if err := rows.Scan(&raw); err != nil {
			return nil, err
		}
		var k Key
		copy(k[:], raw)
		out = append(out, k)
	}
	return out, rows.Err()
}

// DeleteSpace deletes a space and everything in it, recording it in the
// deletion ledger first.
func (s *Store) DeleteSpace(ctx context.Context, space ID) error {
	return s.tx(ctx, func(tx *sql.Tx) error {
		return s.deleteSpace(ctx, tx, space, true)
	})
}
