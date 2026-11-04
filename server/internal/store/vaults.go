package store

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"errors"
	"time"

	"github.com/lignenative/drywire/server/internal/ledger"
)

// A vault is a sealed copy of an account: its key, name and spaces,
// encrypted by the client under a key derived from a code the server never
// sees (ADR 0051, protocol section 15). The server keeps the sealed bytes
// under the id the same code derives, and hands them to whoever asks with
// that id. It cannot open them.

// VaultID is a vault's id: 32 bytes derived from its code.
type VaultID [32]byte

// VaultKind says what a vault is for.
type VaultKind string

const (
	// VaultRecovery lasts until its account is deleted or makes a new one.
	VaultRecovery VaultKind = "recovery"
	// VaultDevice adds one device: it lasts DeviceVaultLifetime and is
	// gone once opened.
	VaultDevice VaultKind = "device"
)

const (
	// MaxVaultSize bounds a vault's sealed bytes: a few hundred spaces.
	MaxVaultSize = 24 * 1024
	// DeviceVaultLifetime is how long a device code works.
	DeviceVaultLifetime = 10 * time.Minute
	// maxDeviceVaults bounds the device vaults one identity has waiting.
	maxDeviceVaults = 4
)

var (
	ErrVaultTooLarge = errors.New("a vault holds at most 24 KiB")
	ErrTooManyVaults = errors.New("too many device codes waiting: wait for them to expire")
)

func vaultHash(id VaultID) string {
	h := sha256.Sum256(id[:])
	return hex.EncodeToString(h[:])
}

// PutRecoveryVault stores the identity's recovery vault, replacing the one
// it had. A replaced vault is recorded in the ledger, so that restoring an
// older backup cannot bring it back for the old code to open. Storing the
// same id again only updates the sealed bytes.
func (s *Store) PutRecoveryVault(ctx context.Context, identityID int64, id VaultID, sealed []byte) error {
	if len(sealed) > MaxVaultSize {
		return ErrVaultTooLarge
	}
	return s.tx(ctx, func(tx *sql.Tx) error {
		var old []byte
		err := tx.QueryRowContext(ctx, `SELECT id FROM vaults WHERE identity_id = ? AND kind = 'recovery'`,
			identityID).Scan(&old)
		switch {
		case errors.Is(err, sql.ErrNoRows):
		case err != nil:
			return err
		case VaultID(old) == id:
			_, err := tx.ExecContext(ctx, `UPDATE vaults SET sealed = ? WHERE id = ?`, sealed, id[:])
			return err
		default:
			if err := s.forgetVault(ctx, tx, VaultID(old)); err != nil {
				return err
			}
		}
		_, err = tx.ExecContext(ctx,
			`INSERT INTO vaults (id, identity_id, kind, sealed, created_at) VALUES (?, ?, 'recovery', ?, ?)`,
			id[:], identityID, sealed, s.unix())
		if isUniqueViolation(err) {
			// Another identity's vault has this id: as likely as guessing
			// a code. Say nothing about it.
			return ErrNotFound
		}
		return err
	})
}

// DeleteRecoveryVault deletes the identity's recovery vault, if it has one.
func (s *Store) DeleteRecoveryVault(ctx context.Context, identityID int64) error {
	return s.tx(ctx, func(tx *sql.Tx) error {
		var old []byte
		err := tx.QueryRowContext(ctx, `SELECT id FROM vaults WHERE identity_id = ? AND kind = 'recovery'`,
			identityID).Scan(&old)
		if errors.Is(err, sql.ErrNoRows) {
			return nil
		}
		if err != nil {
			return err
		}
		return s.forgetVault(ctx, tx, VaultID(old))
	})
}

func (s *Store) forgetVault(ctx context.Context, tx *sql.Tx, id VaultID) error {
	if err := s.ledger.Append(ledger.Entry{At: s.unix(), Kind: ledger.KindVault, Vault: vaultHash(id)}); err != nil {
		return err
	}
	_, err := tx.ExecContext(ctx, `DELETE FROM vaults WHERE id = ?`, id[:])
	return err
}

// PutDeviceVault stores a vault for one new device, for
// DeviceVaultLifetime. It returns when it expires.
func (s *Store) PutDeviceVault(ctx context.Context, identityID int64, id VaultID, sealed []byte) (time.Time, error) {
	if len(sealed) > MaxVaultSize {
		return time.Time{}, ErrVaultTooLarge
	}
	now := s.unix()
	expires := s.now().Add(DeviceVaultLifetime).Unix()
	err := s.tx(ctx, func(tx *sql.Tx) error {
		if _, err := tx.ExecContext(ctx, `DELETE FROM vaults WHERE expires_at <= ?`, now); err != nil {
			return err
		}
		var waiting int
		if err := tx.QueryRowContext(ctx, `SELECT count(*) FROM vaults WHERE identity_id = ? AND kind = 'device'`,
			identityID).Scan(&waiting); err != nil {
			return err
		}
		if waiting >= maxDeviceVaults {
			return ErrTooManyVaults
		}
		_, err := tx.ExecContext(ctx,
			`INSERT INTO vaults (id, identity_id, kind, sealed, created_at, expires_at) VALUES (?, ?, 'device', ?, ?, ?)`,
			id[:], identityID, sealed, now, expires)
		if isUniqueViolation(err) {
			return ErrNotFound
		}
		return err
	})
	return fromUnix(expires), err
}

// OpenVault returns a vault's sealed bytes and kind. A device vault is
// deleted as it is opened; an expired one is not found.
func (s *Store) OpenVault(ctx context.Context, id VaultID) (sealed []byte, kind VaultKind, err error) {
	err = s.tx(ctx, func(tx *sql.Tx) error {
		var expires sql.NullInt64
		err := tx.QueryRowContext(ctx, `SELECT sealed, kind, expires_at FROM vaults WHERE id = ?`, id[:]).
			Scan(&sealed, &kind, &expires)
		if errors.Is(err, sql.ErrNoRows) {
			return ErrNotFound
		}
		if err != nil {
			return err
		}
		if kind == VaultRecovery {
			return nil
		}
		if _, err := tx.ExecContext(ctx, `DELETE FROM vaults WHERE id = ?`, id[:]); err != nil {
			return err
		}
		if expires.Valid && expires.Int64 <= s.unix() {
			return ErrNotFound
		}
		return nil
	})
	if err != nil {
		return nil, "", err
	}
	return sealed, kind, nil
}

// purgeExpiredVaults deletes the device vaults past their time.
func (s *Store) purgeExpiredVaults(ctx context.Context) (int, error) {
	res, err := s.db.ExecContext(ctx, `DELETE FROM vaults WHERE expires_at <= ?`, s.unix())
	if err != nil {
		return 0, err
	}
	n, _ := res.RowsAffected()
	return int(n), nil
}
