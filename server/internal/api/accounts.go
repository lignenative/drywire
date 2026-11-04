package api

import (
	"errors"
	"fmt"
	"net/http"
	"strings"

	"github.com/lignenative/drywire/server/internal/mail"
	"github.com/lignenative/drywire/server/internal/store"
)

// Accounts by e-mail and user name (ADR 0055, protocol section 13.8): a
// 6-digit code sent to an address signs a device in; a new address then
// chooses its user name; people invite each other by user name.

// emailCode: POST /v1/auth/email {email, lang}: sends a sign-in code. The
// answer is the same whether the address has an account or not.
func (s *Server) emailCode(w http.ResponseWriter, r *http.Request) {
	if s.cfg.Mail == nil {
		writeError(w, http.StatusServiceUnavailable, "this instance sends no e-mail: ask its operator")
		return
	}
	var req struct {
		Email string `json:"email"`
		Lang  string `json:"lang"`
	}
	if !readJSON(w, r, &req) {
		return
	}
	email, err := store.NormalizeEmail(req.Email)
	if err != nil {
		writeError(w, http.StatusBadRequest, err.Error())
		return
	}
	code, err := s.store.NewEmailCode(r.Context(), email)
	if errors.Is(err, store.ErrTooSoon) {
		w.Header().Set("Retry-After", "30")
		writeError(w, http.StatusTooManyRequests, err.Error())
		return
	}
	if err != nil {
		s.fail(w, err)
		return
	}
	if err := s.cfg.Mail.Send(r.Context(), codeMessage(email, code, req.Lang)); err != nil {
		// Neither the address nor the code goes to the log, nor the
		// provider's error, which often quotes the address.
		s.log.Error("sending a sign-in code failed")
		writeError(w, http.StatusBadGateway, "the e-mail could not be sent: try again in a moment")
		return
	}
	writeJSON(w, http.StatusAccepted, map[string]int{"expires_in": int(store.CodeLifetime.Seconds())})
}

// codeMessage is the sign-in e-mail. The code opens the subject, so that
// a notification shows it and iPhone and Mac offer it in the code field
// (autocomplete="one-time-code").
func codeMessage(email, code, lang string) mail.Message {
	if lang == "fr" {
		return mail.Message{
			To:      email,
			Subject: code + " est votre code Drywire",
			Body: fmt.Sprintf("Votre code de connexion Drywire : %s\n\n"+
				"Il est valable 10 minutes. Si vous n’avez rien demandé, ignorez ce message : "+
				"personne n’entrera sans ce code.\n", code),
		}
	}
	return mail.Message{
		To:      email,
		Subject: code + " is your Drywire code",
		Body: fmt.Sprintf("Your Drywire sign-in code: %s\n\n"+
			"It works for 10 minutes. If you did not ask for it, ignore this message: "+
			"nobody gets in without the code.\n", code),
	}
}

// emailVerify: POST /v1/auth/email/verify {email, code, public_key,
// challenge, signature}: the code signs in the device whose key signed
// the challenge, as a login does (section 13.2).
func (s *Server) emailVerify(w http.ResponseWriter, r *http.Request) {
	var req struct {
		Email string `json:"email"`
		Code  string `json:"code"`
		loginRequest
	}
	if !readJSON(w, r, &req) {
		return
	}
	email, err := store.NormalizeEmail(req.Email)
	if err != nil {
		writeError(w, http.StatusBadRequest, err.Error())
		return
	}
	// The key first: a token for a key alone is what any login gives.
	token, expires, ok := s.checkLogin(w, req.loginRequest)
	if !ok {
		return
	}
	// Pasted codes may hold spaces or a dash.
	code := strings.Map(func(r rune) rune {
		if r >= '0' && r <= '9' {
			return r
		}
		return -1
	}, req.Code)
	var wrong store.WrongCodeError
	switch err := s.store.UseEmailCode(r.Context(), email, code); {
	case errors.As(err, &wrong):
		writeJSON(w, http.StatusUnauthorized, map[string]any{"error": err.Error(), "tries_left": wrong.Left})
		return
	case errors.Is(err, store.ErrNoCode):
		writeJSON(w, http.StatusUnauthorized, map[string]any{"error": err.Error(), "tries_left": 0})
		return
	case err != nil:
		s.fail(w, err)
		return
	}
	id, created, err := s.store.SignIn(r.Context(), email, req.PublicKey)
	if errors.Is(err, store.ErrDeviceOwned) {
		s.auth.Revoke(req.PublicKey)
		writeError(w, http.StatusConflict, err.Error())
		return
	}
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{
		"token":       token,
		"expires_in":  int(expires.Sub(s.now()).Seconds()),
		"new_account": created,
		"username":    id.Username,
	})
}

// setUsername: PUT /v1/me/username {username, display_name}.
func (s *Server) setUsername(w http.ResponseWriter, r *http.Request, c caller) {
	id, ok := c.registered(w)
	if !ok {
		return
	}
	var req struct {
		Username    string `json:"username"`
		DisplayName string `json:"display_name"`
	}
	if !readJSON(w, r, &req) {
		return
	}
	updated, err := s.store.SetUsername(r.Context(), id.ID, req.Username, req.DisplayName)
	var taken store.UsernameTakenError
	switch {
	case errors.As(err, &taken):
		writeJSON(w, http.StatusConflict, map[string]any{"error": err.Error(), "suggestions": taken.Suggestions})
	case errors.Is(err, store.ErrBadUsername), errors.Is(err, store.ErrBadName):
		writeError(w, http.StatusBadRequest, err.Error())
	case err != nil:
		s.fail(w, err)
	default:
		writeJSON(w, http.StatusOK, meView(updated))
	}
}

// meView is the caller's own account, address included.
func meView(id store.Identity) map[string]any {
	return map[string]any{
		"public_key":   id.PublicKey,
		"name":         id.Name,
		"username":     id.Username,
		"email":        id.Email,
		"created_at":   id.CreatedAt,
		"last_seen_at": id.LastSeenAt,
	}
}

// devices: GET /v1/me/devices.
func (s *Server) devices(w http.ResponseWriter, r *http.Request, c caller) {
	id, ok := c.registered(w)
	if !ok {
		return
	}
	list, err := s.store.Devices(r.Context(), id.ID)
	if err != nil {
		s.fail(w, err)
		return
	}
	out := make([]map[string]any, 0, len(list))
	for _, d := range list {
		out = append(out, map[string]any{
			"public_key": d.PublicKey, "created_at": d.CreatedAt, "last_seen_at": d.LastSeenAt,
			"this_device": d.PublicKey == c.key,
		})
	}
	writeJSON(w, http.StatusOK, out)
}

// removeDevice: DELETE /v1/me/devices/{key}: signs that device out for
// good, ending its tokens, streams and session.
func (s *Server) removeDevice(w http.ResponseWriter, r *http.Request, c caller) {
	id, ok := c.registered(w)
	if !ok {
		return
	}
	key, ok := pathKey(w, r, "key")
	if !ok {
		return
	}
	if err := s.store.RemoveDevice(r.Context(), id.ID, key); err != nil {
		s.fail(w, err)
		return
	}
	s.auth.Revoke(key)
	s.presence.Kick(nil, key)
	w.WriteHeader(http.StatusNoContent)
}

// user: GET /v1/users/{username}: a person to invite. Only an exact user
// name answers, so the list of accounts cannot be read through it.
func (s *Server) user(w http.ResponseWriter, r *http.Request, c caller) {
	if _, ok := c.registered(w); !ok {
		return
	}
	u, err := s.store.IdentityByUsername(r.Context(), r.PathValue("username"))
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]string{"username": u.Username, "name": u.Name})
}

// inviteUser: POST /v1/spaces/{space}/invitations/users {username}: the
// invitation waits in that person's app.
func (s *Server) inviteUser(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if _, ok := s.role(w, r, c, space, store.RoleAdmin); !ok {
		return
	}
	var req struct {
		Username string `json:"username"`
	}
	if !readJSON(w, r, &req) {
		return
	}
	u, err := s.store.InviteUser(r.Context(), space, c.identity.ID, req.Username)
	if errors.Is(err, store.ErrAlreadyMember) {
		writeError(w, http.StatusConflict, err.Error())
		return
	}
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusCreated, map[string]string{"username": u.Username, "name": u.Name})
}

// myInvitations: GET /v1/me/invitations.
func (s *Server) myInvitations(w http.ResponseWriter, r *http.Request, c caller) {
	id, ok := c.registered(w)
	if !ok {
		return
	}
	list, err := s.store.UserInvitations(r.Context(), id.ID)
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusOK, list)
}

// acceptInvitation: POST /v1/me/invitations/{space}: the space, as a
// redeemed code gives it.
func (s *Server) acceptInvitation(w http.ResponseWriter, r *http.Request, c caller) {
	id, ok := c.registered(w)
	if !ok {
		return
	}
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if err := s.store.AcceptInvitation(r.Context(), space, id.ID); err != nil {
		s.fail(w, err)
		return
	}
	v, err := s.spaceView(r, space, store.RoleMember, true)
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusOK, v)
}

// declineInvitation: DELETE /v1/me/invitations/{space}.
func (s *Server) declineInvitation(w http.ResponseWriter, r *http.Request, c caller) {
	id, ok := c.registered(w)
	if !ok {
		return
	}
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if err := s.store.DeclineInvitation(r.Context(), space, id.ID); err != nil {
		s.fail(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}
