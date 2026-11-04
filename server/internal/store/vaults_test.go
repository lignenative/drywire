package store

import (
	"bytes"
	"context"
	"errors"
	"testing"

	"github.com/lignenative/drywire/server/internal/ledger"
)

func vaultID(n byte) VaultID {
	var id VaultID
	for i := range id {
		id[i] = n
	}
	return id
}

func TestARecoveryVaultLastsAndANewCodeReplacesIt(t *testing.T) {
	s, l, _ := testStore(t)
	ctx := context.Background()
	if _, _, err := s.CreateSpace(ctx, key(1), "alex", "Team", "general", 50); err != nil {
		t.Fatal(err)
	}
	alex := must[Identity](t)(s.IdentityByKey(ctx, key(1)))

	if err := s.PutRecoveryVault(ctx, alex.ID, vaultID(1), []byte("sealed 1")); err != nil {
		t.Fatal(err)
	}
	// Opened as often as needed: it is the person's way back.
	for range 2 {
		sealed, kind, err := s.OpenVault(ctx, vaultID(1))
		if err != nil || kind != VaultRecovery || !bytes.Equal(sealed, []byte("sealed 1")) {
			t.Fatalf("open: %q %q %v", sealed, kind, err)
		}
	}
	// The same code seals it again when the spaces change.
	if err := s.PutRecoveryVault(ctx, alex.ID, vaultID(1), []byte("sealed 2")); err != nil {
		t.Fatal(err)
	}
	if sealed, _, _ := s.OpenVault(ctx, vaultID(1)); !bytes.Equal(sealed, []byte("sealed 2")) {
		t.Fatalf("not updated: %q", sealed)
	}
	// A new code: the old one opens nothing, and the ledger says so.
	if err := s.PutRecoveryVault(ctx, alex.ID, vaultID(2), []byte("sealed 3")); err != nil {
		t.Fatal(err)
	}
	if _, _, err := s.OpenVault(ctx, vaultID(1)); !errors.Is(err, ErrNotFound) {
		t.Fatalf("old code: %v", err)
	}
	entries := must[[]ledger.Entry](t)(l.Entries())
	if len(entries) != 1 || entries[0].Kind != "vault" || entries[0].Vault != vaultHash(vaultID(1)) {
		t.Fatalf("ledger: %+v", entries)
	}
	if err := s.DeleteRecoveryVault(ctx, alex.ID); err != nil {
		t.Fatal(err)
	}
	if _, _, err := s.OpenVault(ctx, vaultID(2)); !errors.Is(err, ErrNotFound) {
		t.Fatalf("deleted: %v", err)
	}
	if err := s.PutRecoveryVault(ctx, alex.ID, vaultID(3), make([]byte, MaxVaultSize+1)); !errors.Is(err, ErrVaultTooLarge) {
		t.Fatalf("too large: %v", err)
	}
}

func TestADeviceVaultOpensOnceWithinTenMinutes(t *testing.T) {
	s, _, clock := testStore(t)
	ctx := context.Background()
	if _, _, err := s.CreateSpace(ctx, key(1), "alex", "Team", "general", 50); err != nil {
		t.Fatal(err)
	}
	alex := must[Identity](t)(s.IdentityByKey(ctx, key(1)))

	if _, err := s.PutDeviceVault(ctx, alex.ID, vaultID(1), []byte("for the phone")); err != nil {
		t.Fatal(err)
	}
	sealed, kind, err := s.OpenVault(ctx, vaultID(1))
	if err != nil || kind != VaultDevice || string(sealed) != "for the phone" {
		t.Fatalf("open: %q %q %v", sealed, kind, err)
	}
	if _, _, err := s.OpenVault(ctx, vaultID(1)); !errors.Is(err, ErrNotFound) {
		t.Fatalf("opened twice: %v", err)
	}

	if _, err := s.PutDeviceVault(ctx, alex.ID, vaultID(2), []byte("late")); err != nil {
		t.Fatal(err)
	}
	clock.advance(DeviceVaultLifetime)
	if _, _, err := s.OpenVault(ctx, vaultID(2)); !errors.Is(err, ErrNotFound) {
		t.Fatalf("expired: %v", err)
	}
	p := must[Purged](t)(s.PurgeInactive(ctx))
	if p.Vaults != 1 {
		t.Fatalf("purged %d vaults, want the expired one", p.Vaults)
	}

	for i := range byte(maxDeviceVaults) {
		if _, err := s.PutDeviceVault(ctx, alex.ID, vaultID(10+i), []byte("x")); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := s.PutDeviceVault(ctx, alex.ID, vaultID(20), []byte("x")); !errors.Is(err, ErrTooManyVaults) {
		t.Fatalf("one too many: %v", err)
	}
}

func TestVaultsGoWithTheirIdentityAndStayGoneAfterARestore(t *testing.T) {
	s, l, clock := testStore(t)
	ctx := context.Background()
	if _, _, err := s.CreateSpace(ctx, key(1), "alex", "Team", "general", 50); err != nil {
		t.Fatal(err)
	}
	alex := must[Identity](t)(s.IdentityByKey(ctx, key(1)))
	if err := s.PutRecoveryVault(ctx, alex.ID, vaultID(1), []byte("old")); err != nil {
		t.Fatal(err)
	}
	e := must[Export](t)(s.Export(ctx, alex.ID))
	if len(e.Vaults) != 1 || e.Vaults[0].Kind != VaultRecovery || e.Vaults[0].Size != 3 {
		t.Fatalf("export: %+v", e.Vaults)
	}

	// A backup taken now still holds vault 1. A new code replaces it.
	clock.advance(1e9)
	if err := s.PutRecoveryVault(ctx, alex.ID, vaultID(2), []byte("new")); err != nil {
		t.Fatal(err)
	}
	// What the restore of that backup brings back: vault 1, as it was.
	if _, err := s.db.ExecContext(ctx, `DELETE FROM vaults`); err != nil {
		t.Fatal(err)
	}
	if _, err := s.db.ExecContext(ctx, `INSERT INTO vaults (id, identity_id, kind, sealed, created_at) VALUES (?, ?, 'recovery', ?, ?)`,
		bytes.Repeat([]byte{1}, 32), alex.ID, []byte("old"), clock.t.Unix()-10); err != nil {
		t.Fatal(err)
	}
	entries := must[[]ledger.Entry](t)(l.Entries())
	if n := must[int](t)(s.Replay(ctx, entries)); n != 1 {
		t.Fatalf("replay applied %d, want the old vault", n)
	}
	if _, _, err := s.OpenVault(ctx, vaultID(1)); !errors.Is(err, ErrNotFound) {
		t.Fatalf("old vault back: %v", err)
	}

	if err := s.PutRecoveryVault(ctx, alex.ID, vaultID(2), []byte("new")); err != nil {
		t.Fatal(err)
	}
	if _, err := s.DeleteIdentity(ctx, alex.ID); err != nil {
		t.Fatal(err)
	}
	if _, _, err := s.OpenVault(ctx, vaultID(2)); !errors.Is(err, ErrNotFound) {
		t.Fatalf("vault outlived its identity: %v", err)
	}
}
