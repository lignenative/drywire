package api

import (
	"encoding/hex"
	"errors"
	"net/http"

	"github.com/lignenative/drywire/server/internal/store"
)

// Vaults carry an account to another device (ADR 0051, protocol section
// 15). The client seals them under a code the server never sees; the
// server stores the sealed bytes under the id the code derives, and gives
// them to whoever sends that id.

type vaultRequest struct {
	ID     string `json:"id"`
	Sealed string `json:"sealed"`
}

// parse checks the id and the sealed bytes, or answers 400.
func (v vaultRequest) parse(w http.ResponseWriter) (store.VaultID, []byte, bool) {
	var id store.VaultID
	b, err := hex.DecodeString(v.ID)
	if err != nil || len(b) != len(id) {
		writeError(w, http.StatusBadRequest, "id: 32 bytes in hex")
		return id, nil, false
	}
	copy(id[:], b)
	sealed, err := hex.DecodeString(v.Sealed)
	if err != nil || len(sealed) == 0 {
		writeError(w, http.StatusBadRequest, "sealed: the vault's bytes in hex")
		return id, nil, false
	}
	return id, sealed, true
}

// putRecoveryVault: PUT /v1/me/vault {id, sealed}, the caller's recovery
// vault, replacing the one it had.
func (s *Server) putRecoveryVault(w http.ResponseWriter, r *http.Request, c caller) {
	me, ok := c.registered(w)
	if !ok {
		return
	}
	var req vaultRequest
	if !readJSON(w, r, &req) {
		return
	}
	id, sealed, ok := req.parse(w)
	if !ok {
		return
	}
	if err := s.store.PutRecoveryVault(r.Context(), me.ID, id, sealed); err != nil {
		s.fail(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

// deleteRecoveryVault: DELETE /v1/me/vault. The recovery code opens
// nothing on this instance any more.
func (s *Server) deleteRecoveryVault(w http.ResponseWriter, r *http.Request, c caller) {
	me, ok := c.registered(w)
	if !ok {
		return
	}
	if err := s.store.DeleteRecoveryVault(r.Context(), me.ID); err != nil {
		s.fail(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

// putDeviceVault: POST /v1/me/device-vaults {id, sealed}, a vault for one
// new device, for ten minutes.
func (s *Server) putDeviceVault(w http.ResponseWriter, r *http.Request, c caller) {
	me, ok := c.registered(w)
	if !ok {
		return
	}
	var req vaultRequest
	if !readJSON(w, r, &req) {
		return
	}
	id, sealed, ok := req.parse(w)
	if !ok {
		return
	}
	expires, err := s.store.PutDeviceVault(r.Context(), me.ID, id, sealed)
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusCreated, map[string]any{
		"expires_in": int(store.DeviceVaultLifetime.Seconds()),
		"expires_at": expires,
	})
}

// openVault: POST /v1/vaults/open {id} → {kind, sealed}. No login: the
// device asking has no account yet. Every failure is the same 404.
func (s *Server) openVault(w http.ResponseWriter, r *http.Request) {
	var req struct {
		ID string `json:"id"`
	}
	if !readJSON(w, r, &req) {
		return
	}
	var id store.VaultID
	b, err := hex.DecodeString(req.ID)
	if err != nil || len(b) != len(id) {
		writeError(w, http.StatusNotFound, "no vault for this code")
		return
	}
	copy(id[:], b)
	sealed, kind, err := s.store.OpenVault(r.Context(), id)
	if errors.Is(err, store.ErrNotFound) {
		writeError(w, http.StatusNotFound, "no vault for this code")
		return
	}
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"kind": kind, "sealed": hex.EncodeToString(sealed)})
}
