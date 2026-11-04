package api

import (
	"context"
	"encoding/hex"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/lignenative/drywire/server/internal/store"
)

func TestAVaultCarriesAnAccountToANewDevice(t *testing.T) {
	h := newHarness(t, Config{AnyoneCreates: true})
	alex := h.client(1).login()
	id := strings.Repeat("ab", 32)
	sealed := hex.EncodeToString([]byte("sealed on the device"))

	// A key with no identity here has nothing to keep a vault for.
	if status := alex.do("PUT", "/v1/me/vault", map[string]string{"id": id, "sealed": sealed}, nil); status != http.StatusForbidden {
		t.Fatalf("no identity yet: %d", status)
	}
	alex.createSpace("Team")
	alex.must(http.StatusNoContent, "PUT", "/v1/me/vault", map[string]string{"id": id, "sealed": sealed}, nil)
	if status := alex.do("PUT", "/v1/me/vault", map[string]string{"id": "abcd", "sealed": sealed}, nil); status != http.StatusBadRequest {
		t.Fatalf("short id: %d", status)
	}

	// The new device has no key and no token: it only has the code.
	phone := h.client(9)
	h.clock.advance(time.Second)
	var got struct {
		Kind   string `json:"kind"`
		Sealed string `json:"sealed"`
	}
	phone.must(http.StatusOK, "POST", "/v1/vaults/open", map[string]string{"id": id}, &got)
	if got.Kind != "recovery" || got.Sealed != sealed {
		t.Fatalf("opened %+v", got)
	}
	h.clock.advance(time.Second)
	if status := phone.do("POST", "/v1/vaults/open", map[string]string{"id": strings.Repeat("cd", 32)}, nil); status != http.StatusNotFound {
		t.Fatalf("unknown id: %d", status)
	}

	// A device code: once, then gone.
	device := strings.Repeat("ef", 32)
	var created struct {
		ExpiresIn int `json:"expires_in"`
	}
	alex.must(http.StatusCreated, "POST", "/v1/me/device-vaults", map[string]string{"id": device, "sealed": sealed}, &created)
	if created.ExpiresIn != 600 {
		t.Fatalf("expires_in %d", created.ExpiresIn)
	}
	h.clock.advance(time.Second)
	phone.must(http.StatusOK, "POST", "/v1/vaults/open", map[string]string{"id": device}, &got)
	if got.Kind != "device" {
		t.Fatalf("kind %q", got.Kind)
	}
	h.clock.advance(time.Second)
	if status := phone.do("POST", "/v1/vaults/open", map[string]string{"id": device}, nil); status != http.StatusNotFound {
		t.Fatalf("opened twice: %d", status)
	}

	var export map[string]any
	alex.must(http.StatusOK, "GET", "/v1/me/export", nil, &export)
	if vaults, _ := export["vaults"].([]any); len(vaults) != 1 {
		t.Fatalf("export vaults: %v", export["vaults"])
	}
	alex.must(http.StatusNoContent, "DELETE", "/v1/me/vault", nil, nil)
	h.clock.advance(time.Second)
	if status := phone.do("POST", "/v1/vaults/open", map[string]string{"id": id}, nil); status != http.StatusNotFound {
		t.Fatalf("deleted vault: %d", status)
	}
}

func TestOpeningVaultsIsPacedLikeLogins(t *testing.T) {
	h := newHarness(t, Config{})
	guesser := h.client(1)
	id := map[string]string{"id": strings.Repeat("00", 32)}
	limited := false
	for range 20 {
		if guesser.do("POST", "/v1/vaults/open", id, nil) == http.StatusTooManyRequests {
			limited = true
			break
		}
	}
	if !limited {
		t.Fatal("twenty guesses in a row were all answered")
	}
}

func TestACreationCodeLetsAnUnlistedKeyCreateOneSpace(t *testing.T) {
	h := newHarness(t, Config{})
	sam := h.client(2).login()
	body := map[string]string{"name": "Scrims", "display_name": "sam"}
	if status := sam.do("POST", "/v1/spaces", body, nil); status != http.StatusForbidden {
		t.Fatalf("no code: %d", status)
	}
	code := store.NewCreatorCode()
	if _, err := h.store.AddCreatorCode(context.Background(), code, store.CreatorCodeLifetime); err != nil {
		t.Fatal(err)
	}
	body["creator_code"] = "not a code"
	if status := sam.do("POST", "/v1/spaces", body, nil); status != http.StatusBadRequest {
		t.Fatalf("bad code: %d", status)
	}
	body["creator_code"] = strings.ToLower(code.String())
	var v SpaceView
	sam.must(http.StatusCreated, "POST", "/v1/spaces", body, &v)
	if v.Role != "owner" {
		t.Fatalf("role %q", v.Role)
	}
	if status := sam.do("POST", "/v1/spaces", body, nil); status != http.StatusNotFound {
		t.Fatalf("code used twice: %d", status)
	}
}
