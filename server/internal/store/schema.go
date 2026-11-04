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
	// 2: vaults, the sealed copies of an account that let another device
	// become it (ADR 0051). The server cannot open them.
	`
CREATE TABLE vaults (
	id          BLOB    PRIMARY KEY CHECK (length(id) = 32),
	identity_id INTEGER NOT NULL REFERENCES identities (id) ON DELETE CASCADE,
	kind        TEXT    NOT NULL CHECK (kind IN ('recovery', 'device')),
	sealed      BLOB    NOT NULL,
	created_at  INTEGER NOT NULL,
	expires_at  INTEGER  -- NULL for a recovery vault, which lasts
);
CREATE INDEX vaults_by_identity ON vaults (identity_id);
-- One recovery vault per identity: a new code replaces the old one.
CREATE UNIQUE INDEX one_recovery_vault ON vaults (identity_id) WHERE kind = 'recovery';
`,
	// 3: creation codes, which let one person create one space without
	// being listed in DRYWIRE_SPACE_CREATORS (ADR 0054). Only their SHA-256
	// is kept, as for invitations, and nothing about who used them.
	`
CREATE TABLE creator_codes (
	code_hash  BLOB    PRIMARY KEY CHECK (length(code_hash) = 32),
	created_at INTEGER NOT NULL,
	expires_at INTEGER NOT NULL
);
`,
	// 4: the MLS delivery service (ADR 0052): opaque bytes the server
	// stores and orders, never reads. A device is named by its MLS public
	// key, since a person's devices share their account key (ADR 0051).
	`
CREATE TABLE mls_key_packages (
	space_id    BLOB    NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
	device      BLOB    NOT NULL CHECK (length(device) = 32),
	identity_id INTEGER NOT NULL REFERENCES identities (id) ON DELETE CASCADE,
	body        BLOB    NOT NULL,
	created_at  INTEGER NOT NULL,
	PRIMARY KEY (space_id, device)
);
CREATE TABLE mls_commits (
	space_id   BLOB    NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
	epoch      INTEGER NOT NULL, -- the epoch this commit starts
	body       BLOB    NOT NULL,
	welcome    BLOB,             -- for the devices it adds, one message for all
	created_at INTEGER NOT NULL,
	PRIMARY KEY (space_id, epoch)
);
CREATE TABLE mls_welcomes (
	space_id    BLOB    NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
	device      BLOB    NOT NULL CHECK (length(device) = 32),
	identity_id INTEGER NOT NULL REFERENCES identities (id) ON DELETE CASCADE,
	epoch       INTEGER NOT NULL, -- the commit whose welcome it is
	created_at  INTEGER NOT NULL,
	PRIMARY KEY (space_id, device)
);
`,
	// 5: accounts by e-mail and user name (ADR 0055). An identity becomes
	// an account; each device logs in with a key of its own, listed in
	// devices. identities.public_key stays the first device's, as the
	// deletion ledger knows identities by it.
	`
ALTER TABLE identities ADD COLUMN email TEXT;    -- lower case; NULL before ADR 0055
ALTER TABLE identities ADD COLUMN username TEXT; -- lower case; NULL until chosen
CREATE UNIQUE INDEX identities_by_email ON identities (email) WHERE email IS NOT NULL;
CREATE UNIQUE INDEX identities_by_username ON identities (username) WHERE username IS NOT NULL;

CREATE TABLE devices (
	public_key   BLOB    PRIMARY KEY CHECK (length(public_key) = 32),
	identity_id  INTEGER NOT NULL REFERENCES identities (id) ON DELETE CASCADE,
	created_at   INTEGER NOT NULL,
	last_seen_at INTEGER NOT NULL
);
CREATE INDEX devices_by_identity ON devices (identity_id);
INSERT INTO devices (public_key, identity_id, created_at, last_seen_at)
	SELECT public_key, id, created_at, last_seen_at FROM identities;

-- A sign-in code: only its hash, for 10 minutes and 5 tries.
CREATE TABLE email_codes (
	email      TEXT    PRIMARY KEY,
	code_hash  BLOB    NOT NULL CHECK (length(code_hash) = 32),
	tries      INTEGER NOT NULL DEFAULT 0,
	created_at INTEGER NOT NULL,
	expires_at INTEGER NOT NULL
);

-- An invitation to a person by user name, waiting in their app.
CREATE TABLE user_invitations (
	space_id    BLOB    NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
	identity_id INTEGER NOT NULL REFERENCES identities (id) ON DELETE CASCADE,
	invited_by  INTEGER REFERENCES identities (id) ON DELETE SET NULL,
	created_at  INTEGER NOT NULL,
	expires_at  INTEGER NOT NULL,
	PRIMARY KEY (space_id, identity_id)
);
CREATE INDEX user_invitations_by_identity ON user_invitations (identity_id);
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
