package store

import (
	"bytes"
	"context"
	"errors"
	"testing"
	"time"
)

func device(n byte) Device {
	var d Device
	for i := range d {
		d[i] = n
	}
	return d
}

func TestTheGroupTakesOneCommitPerEpoch(t *testing.T) {
	s, _, _ := testStore(t)
	ctx := context.Background()
	sp, _ := space(t, s)
	alex := identity(t, s, key(1))
	sam := join(t, s, sp, key(2), "sam")

	if epoch := must[uint64](t)(s.MLSEpoch(ctx, sp.ID)); epoch != 0 {
		t.Fatalf("a new group's epoch: %d", epoch)
	}
	// Sam's phone waits to join; posting again replaces its key package.
	if err := s.PutKeyPackage(ctx, sp.ID, sam.ID, device(2), []byte("old")); err != nil {
		t.Fatal(err)
	}
	if err := s.PutKeyPackage(ctx, sp.ID, sam.ID, device(2), []byte("kp")); err != nil {
		t.Fatal(err)
	}
	// Nobody else posts for that device.
	if err := s.PutKeyPackage(ctx, sp.ID, alex.ID, device(2), []byte("mine")); !errors.Is(err, ErrDeviceTaken) {
		t.Fatalf("someone else's device: %v", err)
	}
	pending := must[[]PendingDevice](t)(s.KeyPackages(ctx, sp.ID))
	if len(pending) != 1 || pending[0].Device != device(2) || pending[0].PublicKey != key(2) || string(pending[0].KeyPackage) != "kp" {
		t.Fatalf("pending %+v", pending)
	}
	if err := s.PutKeyPackage(ctx, sp.ID, sam.ID, device(2), make([]byte, MaxMLSMessage+1)); !errors.Is(err, ErrMLSTooLarge) {
		t.Fatalf("too large: %v", err)
	}

	// Alex's commit creates epoch 1 and welcomes the phone.
	next, err := s.Commit(ctx, sp.ID, 0, []byte("c1"), []Welcome{{Device: device(2), Body: []byte("w")}})
	if err != nil || next != 1 {
		t.Fatalf("commit: %d %v", next, err)
	}
	if pending := must[[]PendingDevice](t)(s.KeyPackages(ctx, sp.ID)); len(pending) != 0 {
		t.Fatalf("still pending: %+v", pending)
	}
	// A second commit from epoch 0 lost the race.
	var stale StaleEpochError
	if _, err := s.Commit(ctx, sp.ID, 0, []byte("late"), nil); !errors.As(err, &stale) || stale.Current != 1 {
		t.Fatalf("stale commit: %v", err)
	}
	// A welcome for a device that never asked is refused, and the commit
	// with it.
	if _, err := s.Commit(ctx, sp.ID, 1, []byte("c2"), []Welcome{{Device: device(9), Body: []byte("w")}}); !errors.Is(err, ErrNotFound) {
		t.Fatalf("welcome for no one: %v", err)
	}
	if epoch := must[uint64](t)(s.MLSEpoch(ctx, sp.ID)); epoch != 1 {
		t.Fatalf("epoch after a refused commit: %d", epoch)
	}
	if _, err := s.Commit(ctx, sp.ID, 1, []byte("c2"), nil); err != nil {
		t.Fatal(err)
	}
	commits := must[[]CommitRow](t)(s.Commits(ctx, sp.ID, 0))
	if len(commits) != 2 || commits[0].Epoch != 1 || commits[1].Epoch != 2 || !bytes.Equal(commits[1].Body, []byte("c2")) {
		t.Fatalf("commits %+v", commits)
	}
	if commits := must[[]CommitRow](t)(s.Commits(ctx, sp.ID, 1)); len(commits) != 1 || commits[0].Epoch != 2 {
		t.Fatalf("commits after 1: %+v", commits)
	}

	// Nor once it is welcomed.
	if err := s.PutKeyPackage(ctx, sp.ID, alex.ID, device(2), []byte("mine")); !errors.Is(err, ErrDeviceTaken) {
		t.Fatalf("someone else's welcomed device: %v", err)
	}
	// Only the device's own identity reads its welcome.
	if _, _, err := s.WelcomeFor(ctx, sp.ID, alex.ID, device(2)); !errors.Is(err, ErrNotFound) {
		t.Fatalf("someone else's welcome: %v", err)
	}
	epoch, body, err := s.WelcomeFor(ctx, sp.ID, sam.ID, device(2))
	if err != nil || epoch != 1 || string(body) != "w" {
		t.Fatalf("welcome: %d %q %v", epoch, body, err)
	}
}

func TestALeavingMemberTakesTheirMLSRowsAlong(t *testing.T) {
	s, _, _ := testStore(t)
	ctx := context.Background()
	sp, _ := space(t, s)
	sam := join(t, s, sp, key(2), "sam")
	_ = s.PutKeyPackage(ctx, sp.ID, sam.ID, device(2), []byte("kp"))
	_ = s.PutKeyPackage(ctx, sp.ID, sam.ID, device(3), []byte("kp"))
	if _, err := s.Commit(ctx, sp.ID, 0, []byte("c1"), []Welcome{{Device: device(2), Body: []byte("w")}}); err != nil {
		t.Fatal(err)
	}
	export := must[Export](t)(s.Export(ctx, sam.ID))
	if len(export.MLSDevices) != 2 || export.MLSDevices[0].State == export.MLSDevices[1].State {
		t.Fatalf("export: %+v", export.MLSDevices)
	}
	if err := s.RemoveMember(ctx, sp.ID, sam.ID); err != nil {
		t.Fatal(err)
	}
	if pending := must[[]PendingDevice](t)(s.KeyPackages(ctx, sp.ID)); len(pending) != 0 {
		t.Fatalf("pending after leaving: %+v", pending)
	}
	if _, _, err := s.WelcomeFor(ctx, sp.ID, sam.ID, device(2)); !errors.Is(err, ErrNotFound) {
		t.Fatalf("welcome after leaving: %v", err)
	}
	// The commits stay: they are the group's history, which the others
	// still replay.
	if commits := must[[]CommitRow](t)(s.Commits(ctx, sp.ID, 0)); len(commits) != 1 {
		t.Fatalf("commits %+v", commits)
	}
}

func TestTheDeliveryServiceForgetsWhatNobodyTook(t *testing.T) {
	s, _, clock := testStore(t)
	ctx := context.Background()
	sp, _ := space(t, s)
	sam := join(t, s, sp, key(2), "sam")
	_ = s.PutKeyPackage(ctx, sp.ID, sam.ID, device(2), []byte("kp"))
	_ = s.PutKeyPackage(ctx, sp.ID, sam.ID, device(3), []byte("kp"))
	for epoch := uint64(0); epoch < 3; epoch++ {
		var welcomes []Welcome
		if epoch == 0 {
			welcomes = []Welcome{{Device: device(2), Body: []byte("w")}}
		}
		if _, err := s.Commit(ctx, sp.ID, epoch, []byte("c"), welcomes); err != nil {
			t.Fatal(err)
		}
	}
	clock.advance(91 * 24 * time.Hour)
	// The members still log in: only the MLS rows are old.
	for _, k := range []Key{key(1), key(2)} {
		if err := s.Touch(ctx, k); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := s.PurgeInactive(ctx); err != nil {
		t.Fatal(err)
	}
	if pending := must[[]PendingDevice](t)(s.KeyPackages(ctx, sp.ID)); len(pending) != 0 {
		t.Fatalf("pending: %+v", pending)
	}
	if _, _, err := s.WelcomeFor(ctx, sp.ID, sam.ID, device(2)); !errors.Is(err, ErrNotFound) {
		t.Fatalf("welcome: %v", err)
	}
	// The last commit stays, so the group's epoch does not go back.
	if epoch := must[uint64](t)(s.MLSEpoch(ctx, sp.ID)); epoch != 3 {
		t.Fatalf("epoch %d", epoch)
	}
	if commits := must[[]CommitRow](t)(s.Commits(ctx, sp.ID, 0)); len(commits) != 1 || commits[0].Epoch != 3 {
		t.Fatalf("commits %+v", commits)
	}
}
