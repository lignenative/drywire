package api

import (
	"encoding/hex"
	"errors"
	"net/http"

	"github.com/lignenative/drywire/server/internal/auth"
	"github.com/lignenative/drywire/server/internal/store"
)

// challenge: POST /v1/auth/challenge (protocol section 13.2).
func (s *Server) challenge(w http.ResponseWriter, r *http.Request) {
	c, err := s.auth.Challenge()
	if errors.Is(err, auth.ErrBusy) {
		w.Header().Set("Retry-After", "5")
		writeError(w, http.StatusServiceUnavailable, err.Error())
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{
		"challenge":  hex.EncodeToString(c[:]),
		"expires_in": int(auth.ChallengeLifetime.Seconds()),
		// The origin to sign, so a client can tell a misconfigured
		// server from a bad signature. The client still signs the origin
		// it reached, never this one.
		"audience": s.auth.Audience(),
	})
}

type loginRequest struct {
	PublicKey store.Key `json:"public_key"`
	Challenge string    `json:"challenge"`
	Signature string    `json:"signature"`
}

// login: POST /v1/auth/login.
func (s *Server) login(w http.ResponseWriter, r *http.Request) {
	var req loginRequest
	if !readJSON(w, r, &req) {
		return
	}
	var challenge [auth.ChallengeSize]byte
	raw, err := hex.DecodeString(req.Challenge)
	sig, sigErr := hex.DecodeString(req.Signature)
	if err != nil || sigErr != nil || len(raw) != len(challenge) {
		writeError(w, http.StatusUnauthorized, auth.ErrDenied.Error())
		return
	}
	copy(challenge[:], raw)
	token, expires, err := s.auth.Login(req.PublicKey, challenge, sig)
	if errors.Is(err, auth.ErrBusy) {
		w.Header().Set("Retry-After", "5")
		writeError(w, http.StatusServiceUnavailable, err.Error())
		return
	}
	if err != nil {
		writeError(w, http.StatusUnauthorized, err.Error())
		return
	}
	if err := s.store.Touch(r.Context(), req.PublicKey); err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{
		"token":      token,
		"expires_in": int(expires.Sub(s.now()).Seconds()),
	})
}
