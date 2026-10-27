package store

import (
	"database/sql"
	"fmt"
)

// migrations are the schema changes, in order. The database's user_version
// says how many it has applied. Never edit an applied migration: add a new
// one at the end.
//
// Times are Unix seconds. Keys and ids are raw bytes, checked by length.
var migrations = []string{
	// 1: the phase 2 model.
	`
CREATE TABLE identities (
	id           INTEGER PRIMARY KEY,
	public_key   BLOB    NOT NULL UNIQUE CHECK (length(public_key) = 32),
	name         TEXT    NOT NULL,
	created_at   INTEGER NOT NULL,
	last_seen_at INTEGER NOT NULL  -- last login; drives the inactivity purge
);

CREATE TABLE spaces (
	id          BLOB    PRIMARY KEY CHECK (length(id) = 16),
	name        TEXT    NOT NULL,
	max_members INTEGER NOT NULL,
	created_at  INTEGER NOT NULL
);

CREATE TABLE members (
	space_id    BLOB    NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
	identity_id INTEGER NOT NULL REFERENCES identities (id) ON DELETE CASCADE,
	role        TEXT    NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
	joined_at   INTEGER NOT NULL,
	PRIMARY KEY (space_id, identity_id)
);
CREATE INDEX members_by_identity ON members (identity_id);
-- Exactly one owner per space: at most one here, at least one by the code
-- that removes members.
CREATE UNIQUE INDEX one_owner_per_space ON members (space_id) WHERE role = 'owner';

CREATE TABLE rooms (
	id         BLOB    PRIMARY KEY CHECK (length(id) = 16),
	space_id   BLOB    NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
	name       TEXT    NOT NULL,
	created_at INTEGER NOT NULL,
	UNIQUE (space_id, name)
);

-- A ban names a public key, not an identity: it outlives the identity, so
-- that deleting it and coming back with the same key does not lift it.
CREATE TABLE bans (
	space_id   BLOB    NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
	public_key BLOB    NOT NULL CHECK (length(public_key) = 32),
	created_at INTEGER NOT NULL,
	PRIMARY KEY (space_id, public_key)
);

-- Only the SHA-256 of a code is stored: a copy of the database does not
-- hold a single usable invitation.
CREATE TABLE invitations (
	id         BLOB    PRIMARY KEY CHECK (length(id) = 16),
	code_hash  BLOB    NOT NULL UNIQUE CHECK (length(code_hash) = 32),
	space_id   BLOB    NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
	created_by INTEGER REFERENCES identities (id) ON DELETE SET NULL,
	created_at INTEGER NOT NULL,
	expires_at INTEGER NOT NULL,
	max_uses   INTEGER NOT NULL,
	uses       INTEGER NOT NULL DEFAULT 0
);
`,
}

// migrate applies the migrations the database has not seen yet.
func migrate(db *sql.DB) error {
	var version int
	if err := db.QueryRow(`PRAGMA user_version`).Scan(&version); err != nil {
		return err
	}
	if version > len(migrations) {
		return fmt.Errorf("schema version %d is newer than this server knows (%d): run a newer server", version, len(migrations))
	}
	for i := version; i < len(migrations); i++ {
		tx, err := db.Begin()
		if err != nil {
			return err
		}
		if _, err := tx.Exec(migrations[i]); err != nil {
			tx.Rollback()
			return fmt.Errorf("migration %d: %w", i+1, err)
		}
		// PRAGMA takes no parameters; i+1 is an int, not user input.
		if _, err := tx.Exec(fmt.Sprintf(`PRAGMA user_version = %d`, i+1)); err != nil {
			tx.Rollback()
			return err
		}
		if err := tx.Commit(); err != nil {
			return err
		}
	}
	return nil
}
