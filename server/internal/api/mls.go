package api

import (
	"encoding/hex"
	"errors"
	"net/http"
	"strconv"

	"github.com/lignenative/drywire/server/internal/store"
)

// The MLS delivery service (ADR 0052, protocol section 16): a space's
// members hand each other their group's key packages, commits and
// welcomes through these routes, as opaque bytes in hex. The server
// orders the commits, one per epoch, and reads nothing. Any member may
// use them: the members' apps check what a commit does.

func parseDevice(w http.ResponseWriter, text string) (store.Device, bool) {
	var d store.Device
	b, err := hex.DecodeString(text)
	if err != nil || len(b) != len(d) {
		writeError(w, http.StatusBadRequest, "device: 32 bytes in hex")
		return d, false
	}
	copy(d[:], b)
	return d, true
}

func parseBytes(w http.ResponseWriter, what, text string) ([]byte, bool) {
	b, err := hex.DecodeString(text)
	if err != nil || len(b) == 0 {
		writeError(w, http.StatusBadRequest, what+": bytes in hex")
		return nil, false
	}
	return b, true
}

// putKeyPackage: POST /v1/spaces/{space}/mls/key-packages {device,
// key_package}: this member's device waits to join the group.
func (s *Server) putKeyPackage(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if _, ok := s.role(w, r, c, space, store.RoleMember); !ok {
		return
	}
	var req struct {
		Device     string `json:"device"`
		KeyPackage string `json:"key_package"`
	}
	if !readJSON(w, r, &req) {
		return
	}
	device, ok := parseDevice(w, req.Device)
	if !ok {
		return
	}
	body, ok := parseBytes(w, "key_package", req.KeyPackage)
	if !ok {
		return
	}
	err := s.store.PutKeyPackage(r.Context(), space, c.identity.ID, device, body)
	if errors.Is(err, store.ErrDeviceTaken) {
		writeError(w, http.StatusConflict, err.Error())
		return
	}
	if err != nil {
		s.fail(w, err)
		return
	}
	// The members online add it at once.
	if epoch, err := s.store.MLSEpoch(r.Context(), space); err == nil {
		s.presence.PublishMLS(space, epoch)
	}
	w.WriteHeader(http.StatusNoContent)
}

// listKeyPackages: GET /v1/spaces/{space}/mls/key-packages: the devices
// waiting to join.
func (s *Server) listKeyPackages(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if _, ok := s.role(w, r, c, space, store.RoleMember); !ok {
		return
	}
	pending, err := s.store.KeyPackages(r.Context(), space)
	if err != nil {
		s.fail(w, err)
		return
	}
	out := make([]map[string]string, 0, len(pending))
	for _, p := range pending {
		out = append(out, map[string]string{
			"device":      hex.EncodeToString(p.Device[:]),
			"public_key":  p.PublicKey.String(),
			"key_package": hex.EncodeToString(p.KeyPackage),
		})
	}
	writeJSON(w, http.StatusOK, out)
}

// commitMLS: POST /v1/spaces/{space}/mls/commits {epoch, commit,
// welcomes: [{device, welcome}]}: a commit made in `epoch`, which must be
// the group's current one (409 with the current epoch otherwise).
func (s *Server) commitMLS(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if _, ok := s.role(w, r, c, space, store.RoleMember); !ok {
		return
	}
	var req struct {
		Epoch    uint64 `json:"epoch"`
		Commit   string `json:"commit"`
		Welcomes []struct {
			Device  string `json:"device"`
			Welcome string `json:"welcome"`
		} `json:"welcomes"`
	}
	if !readJSON(w, r, &req) {
		return
	}
	commit, ok := parseBytes(w, "commit", req.Commit)
	if !ok {
		return
	}
	welcomes := make([]store.Welcome, 0, len(req.Welcomes))
	for _, wl := range req.Welcomes {
		device, ok := parseDevice(w, wl.Device)
		if !ok {
			return
		}
		body, ok := parseBytes(w, "welcome", wl.Welcome)
		if !ok {
			return
		}
		welcomes = append(welcomes, store.Welcome{Device: device, Body: body})
	}
	next, err := s.store.Commit(r.Context(), space, req.Epoch, commit, welcomes)
	var stale store.StaleEpochError
	if errors.As(err, &stale) {
		writeJSON(w, http.StatusConflict, map[string]any{"error": stale.Error(), "epoch": stale.Current})
		return
	}
	if errors.Is(err, store.ErrNotFound) {
		writeError(w, http.StatusBadRequest, "a welcome names a device that is not waiting to join")
		return
	}
	if err != nil {
		s.fail(w, err)
		return
	}
	s.presence.PublishMLS(space, next)
	writeJSON(w, http.StatusCreated, map[string]uint64{"epoch": next})
}

// listCommits: GET /v1/spaces/{space}/mls/commits?after=N: the commits that
// started the epochs after N, in order, 64 at most.
func (s *Server) listCommits(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if _, ok := s.role(w, r, c, space, store.RoleMember); !ok {
		return
	}
	after := uint64(0)
	if v := r.URL.Query().Get("after"); v != "" {
		n, err := strconv.ParseUint(v, 10, 64)
		if err != nil {
			writeError(w, http.StatusBadRequest, "after: an epoch number")
			return
		}
		after = n
	}
	commits, err := s.store.Commits(r.Context(), space, after)
	if err != nil {
		s.fail(w, err)
		return
	}
	out := make([]map[string]any, 0, len(commits))
	for _, cm := range commits {
		out = append(out, map[string]any{"epoch": cm.Epoch, "commit": hex.EncodeToString(cm.Body)})
	}
	writeJSON(w, http.StatusOK, out)
}

// welcome: GET /v1/spaces/{space}/mls/welcome?device=…: the welcome this
// member's device was given, and the epoch it joins.
func (s *Server) welcome(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if _, ok := s.role(w, r, c, space, store.RoleMember); !ok {
		return
	}
	device, ok := parseDevice(w, r.URL.Query().Get("device"))
	if !ok {
		return
	}
	epoch, body, err := s.store.WelcomeFor(r.Context(), space, c.identity.ID, device)
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"epoch": epoch, "welcome": hex.EncodeToString(body)})
}
