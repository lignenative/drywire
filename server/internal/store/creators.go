package store

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"errors"
	"strings"
	"time"
)

// A creation code lets one person create one space on an instance whose
// operator did not list their key (ADR 0054). The operator makes it with
// `drywire-server creator-code` and hands it over; the server keeps only
// its SHA-256, and forgets it once used or expired.

// CreatorCode is a creation code's 16 secret bytes.
type CreatorCode [16]byte

// CreatorCodeLifetime is how long a creation code works by default.
const CreatorCodeLifetime = 7 * 24 * time.Hour

var (
	ErrInvalidCreatorCode = errors.New("this creation code is unknown, already used or expired")
	ErrBadCreatorCode     = errors.New("this is not a Drywire code")
)

// crockford is Crockford's base32 alphabet, the one vault codes use
// (protocol section 15.1), so that every code a person types reads the
// same way.
const crockford = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"

// NewCreatorCode draws a code from the system's random source.
func NewCreatorCode() CreatorCode {
	var c CreatorCode
	if _, err := rand.Read(c[:]); err != nil {
		panic(err) // crypto/rand never fails on a supported system
	}
	return c
}

// String writes the code as people see it: 26 characters in groups of four.
func (c CreatorCode) String() string {
	var b strings.Builder
	// 128 bits, then two zero bits: 26 characters of 5 bits.
	bit := func(i int) int {
		if i >= 128 {
			return 0
		}
		return int(c[i/8]>>(7-i%8)) & 1
	}
	for ch := 0; ch < 26; ch++ {
		if ch > 0 && ch%4 == 0 {
			b.WriteByte('-')
		}
		v := 0
		for k := 0; k < 5; k++ {
			v = v<<1 | bit(ch*5+k)
		}
		b.WriteByte(crockford[v])
	}
	return b.String()
}

// ParseCreatorCode reads a code as a person typed it: any case, hyphens and
// spaces ignored, O read as 0 and I or L as 1.
func ParseCreatorCode(text string) (CreatorCode, error) {
	var c CreatorCode
	var values []int
	for _, r := range strings.ToUpper(text) {
		switch r {
		case '-', ' ':
			continue
		case 'O':
			r = '0'
		case 'I', 'L':
			r = '1'
		}
		v := strings.IndexRune(crockford, r)
		if v < 0 {
			return c, ErrBadCreatorCode
		}
		values = append(values, v)
	}
	if len(values) != 26 || values[25]&0b11 != 0 {
		return c, ErrBadCreatorCode
	}
	for i := 0; i < 128; i++ {
		v := values[i/5] >> (4 - i%5) & 1
		c[i/8] |= byte(v) << (7 - i%8)
	}
	return c, nil
}

func creatorHash(c CreatorCode) []byte {
	h := sha256.Sum256(c[:])
	return h[:]
}

// AddCreatorCode keeps a new code's hash until it expires.
func (s *Store) AddCreatorCode(ctx context.Context, code CreatorCode, lifetime time.Duration) (time.Time, error) {
	now := s.now()
	expires := now.Add(lifetime)
	_, err := s.db.ExecContext(ctx,
		`INSERT INTO creator_codes (code_hash, created_at, expires_at) VALUES (?, ?, ?)`,
		creatorHash(code), now.Unix(), expires.Unix())
	return fromUnix(expires.Unix()), err
}

// useCreatorCode deletes a code that can still be used, or answers
// ErrInvalidCreatorCode.
func useCreatorCode(ctx context.Context, tx *sql.Tx, code CreatorCode, now int64) error {
	res, err := tx.ExecContext(ctx, `DELETE FROM creator_codes WHERE code_hash = ? AND expires_at > ?`,
		creatorHash(code), now)
	if err != nil {
		return err
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return ErrInvalidCreatorCode
	}
	return nil
}

// purgeExpiredCreatorCodes deletes the codes past their time.
func (s *Store) purgeExpiredCreatorCodes(ctx context.Context) error {
	_, err := s.db.ExecContext(ctx, `DELETE FROM creator_codes WHERE expires_at <= ?`, s.unix())
	return err
}
