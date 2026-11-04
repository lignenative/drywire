package store

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"database/sql"
	"errors"
	"fmt"
	"math/big"
	"strings"
	"time"
)

// Accounts by e-mail and user name (ADR 0055): a 6-digit code sent to the
// address signs a device in; a new address then chooses its user name.

const (
	// CodeLifetime is how long a sign-in code works.
	CodeLifetime = 10 * time.Minute
	// CodeTries is how many wrong codes end one.
	CodeTries = 5
	// codeEvery is the least time between two codes for one address, so
	// nobody fills someone's mailbox.
	codeEvery = 30 * time.Second
	// InvitationLifetime is how long an invitation by user name waits.
	InvitationLifetime = 30 * 24 * time.Hour
)

var (
	// ErrBadEmail is not an e-mail address.
	ErrBadEmail = errors.New("not an e-mail address")
	// ErrBadUsername breaks the user name rules.
	ErrBadUsername = errors.New("a user name is 2 to 32 letters, digits, _ and dots")
	// ErrTooSoon asks for a new code within codeEvery of the last.
	ErrTooSoon = errors.New("a code was sent a moment ago: wait before asking again")
	// ErrNoCode is a code that does not match, ran out of tries or
	// expired; WrongCodeError says how many tries are left.
	ErrNoCode = errors.New("no such code: ask for a new one")
	// ErrDeviceOwned is a device key another account uses.
	ErrDeviceOwned = errors.New("this device is signed in to another account")
	// ErrNoUsername is an account that has not chosen its user name yet.
	ErrNoUsername = errors.New("choose a user name first")
	// ErrAlreadyMember is an invitation for someone in the space already.
	ErrAlreadyMember = errors.New("already a member")
)

// UsernameTakenError is a user name someone has, with free ones like it.
type UsernameTakenError struct{ Suggestions []string }

func (e UsernameTakenError) Error() string { return "this user name is taken" }

// WrongCodeError is a code that does not match, with the tries left.
type WrongCodeError struct{ Left int }

func (e WrongCodeError) Error() string {
	return fmt.Sprintf("this code does not match: %d tries left", e.Left)
}

// NormalizeEmail returns the address in lower case, or ErrBadEmail. It
// checks the shape only: the code sent there proves the rest.
func NormalizeEmail(email string) (string, error) {
	email = strings.ToLower(strings.TrimSpace(email))
	at := strings.LastIndexByte(email, '@')
	if len(email) > 254 || at < 1 || at == len(email)-1 || !strings.Contains(email[at:], ".") ||
		strings.ContainsAny(email, " \t\r\n<>,;\"") {
		return "", ErrBadEmail
	}
	return email, nil
}

// NormalizeUsername returns the user name in lower case, or
// ErrBadUsername: 2 to 32 of a-z, 0-9, _ and ., no dot at either end and
// no two in a row, as on Discord.
func NormalizeUsername(name string) (string, error) {
	name = strings.ToLower(strings.TrimSpace(name))
	if len(name) < 2 || len(name) > 32 || name[0] == '.' || name[len(name)-1] == '.' || strings.Contains(name, "..") {
		return "", ErrBadUsername
	}
	for _, r := range name {
		if !(r >= 'a' && r <= 'z' || r >= '0' && r <= '9' || r == '_' || r == '.') {
			return "", ErrBadUsername
		}
	}
	return name, nil
}

func emailCodeHash(email, code string) []byte {
	h := sha256.Sum256([]byte("drywire/email-code/v1\x00" + email + "\x00" + code))
	return h[:]
}

// NewEmailCode draws the sign-in code for this address, replacing any
// earlier one. Only its hash is kept; the caller sends the code.
func (s *Store) NewEmailCode(ctx context.Context, email string) (string, error) {
	n, err := rand.Int(rand.Reader, big.NewInt(1_000_000))
	if err != nil {
		return "", err
	}
	code := fmt.Sprintf("%06d", n.Int64())
	err = s.tx(ctx, func(tx *sql.Tx) error {
		var created int64
		err := tx.QueryRowContext(ctx, `SELECT created_at FROM email_codes WHERE email = ?`, email).Scan(&created)
		if err == nil && s.now().Sub(fromUnix(created)) < codeEvery {
			return ErrTooSoon
		}
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		now := s.now()
		_, err = tx.ExecContext(ctx, `
			INSERT INTO email_codes (email, code_hash, tries, created_at, expires_at) VALUES (?, ?, 0, ?, ?)
			ON CONFLICT (email) DO UPDATE SET code_hash = excluded.code_hash, tries = 0,
				created_at = excluded.created_at, expires_at = excluded.expires_at`,
			email, emailCodeHash(email, code), now.Unix(), now.Add(CodeLifetime).Unix())
		return err
	})
	return code, err
}

// UseEmailCode checks a code typed for this address. A right one is used
// up; a wrong one spends a try (WrongCodeError), and the last try or an
// expired code leaves nothing (ErrNoCode).
func (s *Store) UseEmailCode(ctx context.Context, email, code string) error {
	return s.tx(ctx, func(tx *sql.Tx) error {
		var hash []byte
		var tries int
		var expires int64
		err := tx.QueryRowContext(ctx, `SELECT code_hash, tries, expires_at FROM email_codes WHERE email = ?`, email).
			Scan(&hash, &tries, &expires)
		if errors.Is(err, sql.ErrNoRows) {
			return ErrNoCode
		}
		if err != nil {
			return err
		}
		if s.unix() >= expires {
			_, err := tx.ExecContext(ctx, `DELETE FROM email_codes WHERE email = ?`, email)
			if err != nil {
				return err
			}
			return ErrNoCode
		}
		if subtle.ConstantTimeCompare(hash, emailCodeHash(email, code)) == 1 {
			_, err := tx.ExecContext(ctx, `DELETE FROM email_codes WHERE email = ?`, email)
			return err
		}
		tries++
		if tries >= CodeTries {
			if _, err := tx.ExecContext(ctx, `DELETE FROM email_codes WHERE email = ?`, email); err != nil {
				return err
			}
			// Committed: the spent code stays spent.
			return wrongLast{}
		}
		if _, err := tx.ExecContext(ctx, `UPDATE email_codes SET tries = ? WHERE email = ?`, tries, email); err != nil {
			return err
		}
		return wrongTry{left: CodeTries - tries}
	})
}

// The transaction commits these, unlike an error: a wrong try counts.
type wrongTry struct{ left int }
type wrongLast struct{}

func (w wrongTry) Error() string   { return WrongCodeError{Left: w.left}.Error() }
func (wrongLast) Error() string    { return ErrNoCode.Error() }
func (w wrongTry) Unwrap() error   { return WrongCodeError{Left: w.left} }
func (wrongLast) Unwrap() error    { return ErrNoCode }
func (w wrongTry) committed() bool { return true }
func (wrongLast) committed() bool  { return true }

// SignIn makes this device one of the account at this address, creating
// the account if there is none. It returns the account and whether it is
// new. A device that already had an account without an address, from
// before ADR 0055, brings it along: its spaces stay its own.
func (s *Store) SignIn(ctx context.Context, email string, device Key) (Identity, bool, error) {
	var out Identity
	var created bool
	err := s.tx(ctx, func(tx *sql.Tx) error {
		mine, err := scanIdentity(tx.QueryRowContext(ctx,
			`SELECT `+identityColumns+` FROM devices d JOIN identities i ON i.id = d.identity_id WHERE d.public_key = ?`, device[:]))
		if err != nil && !errors.Is(err, ErrNotFound) {
			return err
		}
		owner, err := scanIdentity(tx.QueryRowContext(ctx,
			`SELECT `+identityColumns+` FROM identities i WHERE i.email = ?`, email))
		if err != nil && !errors.Is(err, ErrNotFound) {
			return err
		}
		now := s.unix()
		switch {
		case owner.ID != 0 && mine.ID == owner.ID:
			out = owner // signed in again on the same device
		case owner.ID != 0 && mine.ID != 0:
			return ErrDeviceOwned
		case owner.ID != 0:
			if _, err := tx.ExecContext(ctx,
				`INSERT INTO devices (public_key, identity_id, created_at, last_seen_at) VALUES (?, ?, ?, ?)`,
				device[:], owner.ID, now, now); err != nil {
				return err
			}
			out = owner
		case mine.ID != 0 && mine.Email != "":
			return ErrDeviceOwned
		case mine.ID != 0:
			if _, err := tx.ExecContext(ctx, `UPDATE identities SET email = ? WHERE id = ?`, email, mine.ID); err != nil {
				return err
			}
			mine.Email = email
			out = mine
		default:
			// The name is the user name's until the person chooses one.
			res, err := tx.ExecContext(ctx,
				`INSERT INTO identities (public_key, name, email, created_at, last_seen_at) VALUES (?, '', ?, ?, ?)`,
				device[:], email, now, now)
			if err != nil {
				return err
			}
			id, _ := res.LastInsertId()
			if _, err := tx.ExecContext(ctx,
				`INSERT INTO devices (public_key, identity_id, created_at, last_seen_at) VALUES (?, ?, ?, ?)`,
				device[:], id, now, now); err != nil {
				return err
			}
			out = Identity{ID: id, PublicKey: device, Email: email, CreatedAt: fromUnix(now), LastSeenAt: fromUnix(now)}
			created = true
		}
		return nil
	})
	return out, created, err
}

// SetUsername gives the account its user name, and its display name when
// one is given (else the user name). A taken name answers
// UsernameTakenError with free ones like it.
func (s *Store) SetUsername(ctx context.Context, identityID int64, username, name string) (Identity, error) {
	username, err := NormalizeUsername(username)
	if err != nil {
		return Identity{}, err
	}
	if strings.TrimSpace(name) == "" {
		name = username
	}
	if name, err = CleanName(name); err != nil {
		return Identity{}, err
	}
	var out Identity
	err = s.tx(ctx, func(tx *sql.Tx) error {
		var other int64
		err := tx.QueryRowContext(ctx, `SELECT id FROM identities WHERE username = ?`, username).Scan(&other)
		if err == nil && other != identityID {
			return UsernameTakenError{Suggestions: s.freeLike(ctx, tx, username)}
		}
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		if _, err := tx.ExecContext(ctx, `UPDATE identities SET username = ?, name = ? WHERE id = ?`,
			username, name, identityID); err != nil {
			return err
		}
		out, err = scanIdentity(tx.QueryRowContext(ctx, `SELECT `+identityColumns+` FROM identities i WHERE i.id = ?`, identityID))
		return err
	})
	return out, err
}

// freeLike finds two free user names close to a taken one.
func (s *Store) freeLike(ctx context.Context, tx *sql.Tx, username string) []string {
	base := username
	if len(base) > 28 {
		base = base[:28]
	}
	out := []string{}
	for tries := 0; len(out) < 2 && tries < 20; tries++ {
		n, _ := rand.Int(rand.Reader, big.NewInt(1000))
		candidate := fmt.Sprintf("%s_%d", strings.TrimRight(base, "."), n.Int64())
		var taken bool
		if err := tx.QueryRowContext(ctx, `SELECT EXISTS (SELECT 1 FROM identities WHERE username = ?)`, candidate).Scan(&taken); err != nil {
			break
		}
		if !taken && candidate != username {
			out = append(out, candidate)
		}
	}
	return out
}

// IdentityByUsername returns the account with this user name, or
// ErrNotFound.
func (s *Store) IdentityByUsername(ctx context.Context, username string) (Identity, error) {
	username, err := NormalizeUsername(username)
	if err != nil {
		return Identity{}, ErrNotFound
	}
	return scanIdentity(s.db.QueryRowContext(ctx, `SELECT `+identityColumns+` FROM identities i WHERE i.username = ?`, username))
}

// AccountDevice is one device of an account.
type AccountDevice struct {
	PublicKey  Key       `json:"public_key"`
	CreatedAt  time.Time `json:"created_at"`
	LastSeenAt time.Time `json:"last_seen_at"`
}

// Devices lists the account's devices, oldest first.
func (s *Store) Devices(ctx context.Context, identityID int64) ([]AccountDevice, error) {
	rows, err := s.db.QueryContext(ctx,
		`SELECT public_key, created_at, last_seen_at FROM devices WHERE identity_id = ? ORDER BY created_at, public_key`, identityID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []AccountDevice{}
	for rows.Next() {
		var d AccountDevice
		var key []byte
		var created, seen int64
		if err := rows.Scan(&key, &created, &seen); err != nil {
			return nil, err
		}
		copy(d.PublicKey[:], key)
		d.CreatedAt, d.LastSeenAt = fromUnix(created), fromUnix(seen)
		out = append(out, d)
	}
	return out, rows.Err()
}

// RemoveDevice signs one of the account's devices out for good. The
// account itself stays, even with no device left: the address signs in
// again.
func (s *Store) RemoveDevice(ctx context.Context, identityID int64, key Key) error {
	res, err := s.db.ExecContext(ctx, `DELETE FROM devices WHERE identity_id = ? AND public_key = ?`, identityID, key[:])
	if err != nil {
		return err
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return ErrNotFound
	}
	return nil
}

// UserInvitation is an invitation waiting in the invited person's app.
type UserInvitation struct {
	SpaceID   ID        `json:"space_id"`
	SpaceName string    `json:"space_name"`
	InvitedBy string    `json:"invited_by"` // a user name, empty if the account is gone
	CreatedAt time.Time `json:"created_at"`
	ExpiresAt time.Time `json:"expires_at"`
}

// InviteUser invites the account with this user name into the space, for
// InvitationLifetime; inviting again renews it.
func (s *Store) InviteUser(ctx context.Context, space ID, by int64, username string) (Identity, error) {
	invitee, err := s.IdentityByUsername(ctx, username)
	if err != nil {
		return Identity{}, err
	}
	err = s.tx(ctx, func(tx *sql.Tx) error {
		var member bool
		if err := tx.QueryRowContext(ctx, `SELECT EXISTS (SELECT 1 FROM members WHERE space_id = ? AND identity_id = ?)`,
			space[:], invitee.ID).Scan(&member); err != nil {
			return err
		}
		if member {
			return ErrAlreadyMember
		}
		now := s.now()
		_, err := tx.ExecContext(ctx, `
			INSERT INTO user_invitations (space_id, identity_id, invited_by, created_at, expires_at) VALUES (?, ?, ?, ?, ?)
			ON CONFLICT (space_id, identity_id) DO UPDATE SET invited_by = excluded.invited_by,
				created_at = excluded.created_at, expires_at = excluded.expires_at`,
			space[:], invitee.ID, by, now.Unix(), now.Add(InvitationLifetime).Unix())
		return err
	})
	return invitee, err
}

// UserInvitations lists the invitations waiting for this account, newest
// first.
func (s *Store) UserInvitations(ctx context.Context, identityID int64) ([]UserInvitation, error) {
	rows, err := s.db.QueryContext(ctx, `
		SELECT u.space_id, sp.name, coalesce(i.username, ''), u.created_at, u.expires_at
		FROM user_invitations u JOIN spaces sp ON sp.id = u.space_id
		LEFT JOIN identities i ON i.id = u.invited_by
		WHERE u.identity_id = ? AND u.expires_at > ? ORDER BY u.created_at DESC`, identityID, s.unix())
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []UserInvitation{}
	for rows.Next() {
		var u UserInvitation
		var space []byte
		var created, expires int64
		if err := rows.Scan(&space, &u.SpaceName, &u.InvitedBy, &created, &expires); err != nil {
			return nil, err
		}
		copy(u.SpaceID[:], space)
		u.CreatedAt, u.ExpiresAt = fromUnix(created), fromUnix(expires)
		out = append(out, u)
	}
	return out, rows.Err()
}

// AcceptInvitation makes the account a member of the space it was invited
// to, as a code would (bans and the member limit hold), and uses the
// invitation up.
func (s *Store) AcceptInvitation(ctx context.Context, space ID, identityID int64) error {
	return s.tx(ctx, func(tx *sql.Tx) error {
		var expires int64
		err := tx.QueryRowContext(ctx, `SELECT expires_at FROM user_invitations WHERE space_id = ? AND identity_id = ?`,
			space[:], identityID).Scan(&expires)
		if errors.Is(err, sql.ErrNoRows) || err == nil && s.unix() >= expires {
			return ErrNotFound
		}
		if err != nil {
			return err
		}
		var banned bool
		if err := tx.QueryRowContext(ctx, `
			SELECT EXISTS (SELECT 1 FROM bans b JOIN devices d ON d.public_key = b.public_key
				WHERE b.space_id = ? AND d.identity_id = ?)`, space[:], identityID).Scan(&banned); err != nil {
			return err
		}
		if banned {
			return ErrBanned
		}
		var count, max int
		if err := tx.QueryRowContext(ctx, `
			SELECT (SELECT count(*) FROM members WHERE space_id = ?), max_members FROM spaces WHERE id = ?`,
			space[:], space[:]).Scan(&count, &max); err != nil {
			return err
		}
		if count >= max {
			return ErrSpaceFull
		}
		if _, err := tx.ExecContext(ctx,
			`INSERT OR IGNORE INTO members (space_id, identity_id, role, joined_at) VALUES (?, ?, 'member', ?)`,
			space[:], identityID, s.unix()); err != nil {
			return err
		}
		_, err = tx.ExecContext(ctx, `DELETE FROM user_invitations WHERE space_id = ? AND identity_id = ?`, space[:], identityID)
		return err
	})
}

// DeclineInvitation forgets an invitation.
func (s *Store) DeclineInvitation(ctx context.Context, space ID, identityID int64) error {
	res, err := s.db.ExecContext(ctx, `DELETE FROM user_invitations WHERE space_id = ? AND identity_id = ?`, space[:], identityID)
	if err != nil {
		return err
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return ErrNotFound
	}
	return nil
}

// purgeExpiredAccountRows deletes the sign-in codes and invitations past
// their time.
func (s *Store) purgeExpiredAccountRows(ctx context.Context) error {
	now := s.unix()
	if _, err := s.db.ExecContext(ctx, `DELETE FROM email_codes WHERE expires_at <= ?`, now); err != nil {
		return err
	}
	_, err := s.db.ExecContext(ctx, `DELETE FROM user_invitations WHERE expires_at <= ?`, now)
	return err
}
