// Package api is the server's HTTP interface. In phase 1 it only tells
// clients and operators what is running; the control plane (identities,
// spaces, rooms) arrives in phase 2.
//
// Responses never contain an address, a session number or a secret.
package api

import (
	"encoding/json"
	"net/http"
	"time"

	"github.com/lignenative/drywire/server/internal/relay"
	"github.com/lignenative/drywire/server/internal/wire"
)

// Version is what GET /v1/version returns. Clients compare
// ProtocolVersions with their own to tell the user when to update
// (protocol section 11).
type Version struct {
	Server string `json:"server"`
	// []int, not []uint8: encoding/json writes a byte slice as base64.
	ProtocolVersions []int `json:"protocol_versions"`
}

// Status is what GET /v1/status returns.
type Status struct {
	Version
	UptimeSeconds int64 `json:"uptime_seconds"`
	relay.Stats
}

// Handler returns the HTTP routes:
//
//	GET /healthz      200 "ok" while the process runs
//	GET /v1/version   server and protocol versions
//	GET /v1/status    uptime and relay counters
func Handler(r *relay.Relay, serverVersion string, started time.Time) http.Handler {
	version := Version{Server: serverVersion}
	for _, v := range wire.SupportedVersions {
		version.ProtocolVersions = append(version.ProtocolVersions, int(v))
	}

	mux := http.NewServeMux()
	mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/plain; charset=utf-8")
		w.Write([]byte("ok\n"))
	})
	mux.HandleFunc("GET /v1/version", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, version)
	})
	mux.HandleFunc("GET /v1/status", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, Status{
			Version:       version,
			UptimeSeconds: int64(time.Since(started).Seconds()),
			Stats:         r.Stats(),
		})
	})
	return mux
}

func writeJSON(w http.ResponseWriter, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	json.NewEncoder(w).Encode(v)
}
