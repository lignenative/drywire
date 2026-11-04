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
// welcomes through these routes, as opaque bytes in hex; their bodies may
// be larger than other requests', for a whole group's tree. The server
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
	if !readJSONUpTo(w, r, &req, 2*store.MaxMLSMessage+maxBody) {
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

// commitMLS: POST /v1/spaces/{space}/mls/commits {epoch, commit, welcome,
// welcomed}: a commit made in `epoch`, which must be the group's current
// one (409 with the current epoch otherwise), and the welcome for the
// devices it adds.
func (s *Server) commitMLS(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if _, ok := s.role(w, r, c, space, store.RoleMember); !ok {
		return
	}
	var req struct {
		Epoch    uint64   `json:"epoch"`
		Commit   string   `json:"commit"`
		Welcome  string   `json:"welcome"`
		Welcomed []string `json:"welcomed"`
	}
	// A commit and a welcome, each up to MaxMLSMessage, in hex.
	if !readJSONUpTo(w, r, &req, 4*store.MaxMLSMessage+maxBody) {
		return
	}
	commit, ok := parseBytes(w, "commit", req.Commit)
	if !ok {
		return
	}
	var welcome []byte
	if req.Welcome != "" {
		if welcome, ok = parseBytes(w, "welcome", req.Welcome); !ok {
			return
		}
	}
	welcomed := make([]store.Device, 0, len(req.Welcomed))
	for _, text := range req.Welcomed {
		device, ok := parseDevice(w, text)
		if !ok {
			return
		}
		welcomed = append(welcomed, device)
	}
	next, err := s.store.Commit(r.Context(), space, req.Epoch, commit, welcome, welcomed)
	var stale store.StaleEpochError
	if errors.As(err, &stale) {
		writeJSON(w, http.StatusConflict, map[string]any{"error": stale.Error(), "epoch": stale.Current})
		return
	}
	if errors.Is(err, store.ErrNotFound) {
		writeError(w, http.StatusBadRequest, "a welcome needs the devices it is for, and each must be waiting to join")
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
