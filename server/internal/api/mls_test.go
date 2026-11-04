package api

import (
	"encoding/hex"
	"net/http"
	"strings"
	"testing"
)

func TestMembersHandEachOtherTheGroupsMessages(t *testing.T) {
	h := newHarness(t, Config{AnyoneCreates: true})
	alex := h.client(1).login()
	v := alex.createSpace("Team")
	base := "/v1/spaces/" + v.Space.ID.String() + "/mls"
	sam := h.client(2).login()
	if status := sam.redeem(alex.invite(v.Space.ID, 1), "sam"); status != http.StatusOK {
		t.Fatalf("redeem: %d", status)
	}
	stranger := h.client(3).login()

	phone := strings.Repeat("02", 32)
	kp := hex.EncodeToString([]byte("key package"))
	sam.must(http.StatusNoContent, "POST", base+"/key-packages", map[string]string{"device": phone, "key_package": kp}, nil)
	if status := sam.do("POST", base+"/key-packages", map[string]string{"device": "02", "key_package": kp}, nil); status != http.StatusBadRequest {
		t.Fatalf("short device: %d", status)
	}
	big := hex.EncodeToString(make([]byte, 16*1024+1))
	if status := sam.do("POST", base+"/key-packages", map[string]string{"device": phone, "key_package": big}, nil); status != http.StatusRequestEntityTooLarge {
		t.Fatalf("too large: %d", status)
	}
	if status := alex.do("POST", base+"/key-packages", map[string]string{"device": phone, "key_package": kp}, nil); status != http.StatusConflict {
		t.Fatalf("someone else's device: %d", status)
	}
	if status := stranger.do("GET", base+"/key-packages", nil, nil); status != http.StatusForbidden && status != http.StatusNotFound {
		t.Fatalf("a stranger lists key packages: %d", status)
	}

	var pending []map[string]string
	alex.must(http.StatusOK, "GET", base+"/key-packages", nil, &pending)
	if len(pending) != 1 || pending[0]["device"] != phone || pending[0]["key_package"] != kp || pending[0]["public_key"] == "" {
		t.Fatalf("pending %v", pending)
	}

	commit := map[string]any{
		"epoch":    0,
		"commit":   hex.EncodeToString([]byte("commit 1")),
		"welcomes": []map[string]string{{"device": phone, "welcome": hex.EncodeToString([]byte("welcome"))}},
	}
	var made struct {
		Epoch uint64 `json:"epoch"`
	}
	alex.must(http.StatusCreated, "POST", base+"/commits", commit, &made)
	if made.Epoch != 1 {
		t.Fatalf("epoch %d", made.Epoch)
	}
	// The same commit again comes too late, and says where the group is.
	var stale struct {
		Epoch uint64 `json:"epoch"`
	}
	if status := alex.do("POST", base+"/commits", map[string]any{"epoch": 0, "commit": "aa"}, &stale); status != http.StatusConflict || stale.Epoch != 1 {
		t.Fatalf("stale: %d %d", status, stale.Epoch)
	}

	var commits []struct {
		Epoch  uint64 `json:"epoch"`
		Commit string `json:"commit"`
	}
	sam.must(http.StatusOK, "GET", base+"/commits?after=0", nil, &commits)
	if len(commits) != 1 || commits[0].Epoch != 1 {
		t.Fatalf("commits %v", commits)
	}
	if status := sam.do("GET", base+"/commits?after=x", nil, nil); status != http.StatusBadRequest {
		t.Fatalf("bad after: %d", status)
	}

	var welcome struct {
		Epoch   uint64 `json:"epoch"`
		Welcome string `json:"welcome"`
	}
	sam.must(http.StatusOK, "GET", base+"/welcome?device="+phone, nil, &welcome)
	if welcome.Epoch != 1 || welcome.Welcome != hex.EncodeToString([]byte("welcome")) {
		t.Fatalf("welcome %+v", welcome)
	}
	if status := alex.do("GET", base+"/welcome?device="+phone, nil, nil); status != http.StatusNotFound {
		t.Fatalf("someone else's welcome: %d", status)
	}
}
