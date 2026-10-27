package api

import (
	"encoding/json"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"github.com/lignenative/drywire/server/internal/relay"
)

func TestRoutes(t *testing.T) {
	r := relay.New(relay.RoomsFile{}, time.Now, slog.New(slog.NewTextHandler(io.Discard, nil)))
	srv := httptest.NewServer(Handler(r, "test", time.Now()))
	defer srv.Close()

	get := func(path string) *http.Response {
		t.Helper()
		resp, err := http.Get(srv.URL + path)
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { resp.Body.Close() })
		return resp
	}

	if resp := get("/healthz"); resp.StatusCode != http.StatusOK {
		t.Fatalf("/healthz: status %d", resp.StatusCode)
	}

	// Compare the raw body: clients in other languages read this JSON.
	body, err := io.ReadAll(get("/v1/version").Body)
	if err != nil {
		t.Fatal(err)
	}
	if got, want := string(body), `{"server":"test","protocol_versions":[1]}`+"\n"; got != want {
		t.Fatalf("/v1/version = %s, want %s", got, want)
	}

	var s map[string]any
	if err := json.NewDecoder(get("/v1/status").Body).Decode(&s); err != nil {
		t.Fatal(err)
	}
	for _, key := range []string{"server", "protocol_versions", "uptime_seconds", "rooms", "bound_sessions", "packets_dropped"} {
		if _, ok := s[key]; !ok {
			t.Errorf("/v1/status has no %q field: %v", key, s)
		}
	}

	if resp := get("/nothing-here"); resp.StatusCode != http.StatusNotFound {
		t.Fatalf("unknown path: status %d, want 404", resp.StatusCode)
	}
}
