package api

import (
	"net/http"
	"testing"

	"github.com/lignenative/drywire/server/internal/ledger"
	"github.com/lignenative/drywire/server/internal/store"
)

func TestTheDataSubjectsRights(t *testing.T) {
	h := newHarness(t, Config{AnyoneCreates: true})
	alex, sam, v := team(t, h)

	// A key that never joined anything has no identity, and nothing to
	// export.
	kim := h.client(3).login()
	if status := kim.do("GET", "/v1/me", nil, nil); status != http.StatusNotFound {
		t.Fatalf("unregistered /v1/me: %d", status)
	}
	if status := kim.do("GET", "/v1/me/export", nil, nil); status != http.StatusForbidden {
		t.Fatalf("unregistered export: %d", status)
	}

	var me store.Identity
	sam.must(200, "PATCH", "/v1/me", map[string]string{"name": "  Sam B "}, &me)
	if me.Name != "Sam B" || me.PublicKey != sam.key {
		t.Fatalf("renamed %+v", me)
	}
	if status := sam.do("PATCH", "/v1/me", map[string]string{"name": "a\x00b"}, nil); status != http.StatusBadRequest {
		t.Fatalf("control character: %d", status)
	}

	var e store.Export
	sam.must(200, "GET", "/v1/me/export", nil, &e)
	if e.Identity.Name != "Sam B" || len(e.Memberships) != 1 || e.Memberships[0].SpaceID != v.Space.ID {
		t.Fatalf("export %+v", e)
	}

	// Leaving a space is deleting one's own membership.
	sam.must(204, "DELETE", "/v1/spaces/"+v.Space.ID.String()+"/members/"+sam.key.String(), nil, nil)
	sam.must(200, "GET", "/v1/me/export", nil, &e)
	if len(e.Memberships) != 0 {
		t.Fatalf("after leaving: %+v", e.Memberships)
	}

	// Deleting the owner hands nothing on here: alex is alone, so the
	// space goes too, and so do alex's session and token.
	a := alex.join(v.Rooms[0].ID)
	alex.must(204, "DELETE", "/v1/me", nil, nil)
	if status := alex.do("GET", "/v1/spaces", nil, nil); status != http.StatusUnauthorized {
		t.Fatalf("token after deletion: %d", status)
	}
	if st := h.relay.Stats(); st.Sessions != 0 {
		t.Fatalf("session %d survived the deletion: %+v", a.Session, st)
	}
	if _, err := h.store.Space(t.Context(), v.Space.ID); err == nil {
		t.Fatal("the space of a deleted owner, alone in it, survived")
	}

	// The ledger names the deleted identity by the hash of its key only.
	l, err := ledger.Open(h.ledgerPath)
	if err != nil {
		t.Fatal(err)
	}
	defer l.Close()
	entries, _ := l.Entries()
	found := false
	for _, e := range entries {
		if e.Kind == ledger.KindIdentity && e.Identity == ledger.KeyHash(alex.key[:]) {
			found = true
		}
	}
	if !found {
		t.Fatalf("no ledger entry for alex: %+v", entries)
	}
}
