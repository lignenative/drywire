package api

import (
	"net/http"

	"github.com/lignenative/drywire/server/internal/presence"
	"github.com/lignenative/drywire/server/internal/store"
)

// The data subject's rights in the app (spec section 9): see and export
// what the server holds, rename, delete everything. Leaving a space is
// DELETE /v1/spaces/{space}/members/{own key}.

// me: GET /v1/me, the caller's identity.
func (s *Server) me(w http.ResponseWriter, r *http.Request, c caller) {
	if c.identity == nil {
		writeError(w, http.StatusNotFound, "this key has no identity on this instance yet")
		return
	}
	writeJSON(w, http.StatusOK, c.identity)
}

// rename: PATCH /v1/me {name}. Rooms the caller is in see the new name
// when it next joins one.
func (s *Server) rename(w http.ResponseWriter, r *http.Request, c caller) {
	id, ok := c.registered(w)
	if !ok {
		return
	}
	var req nameRequest
	if !readJSON(w, r, &req) {
		return
	}
	name, err := s.store.Rename(r.Context(), id.ID, req.Name)
	if err != nil {
		s.fail(w, err)
		return
	}
	id.Name = name
	writeJSON(w, http.StatusOK, id)
}

// export: GET /v1/me/export, everything the server holds about the
// caller, as a JSON file (GDPR articles 15 and 20).
func (s *Server) export(w http.ResponseWriter, r *http.Request, c caller) {
	id, ok := c.registered(w)
	if !ok {
		return
	}
	e, err := s.store.Export(r.Context(), id.ID)
	if err != nil {
		s.fail(w, err)
		return
	}
	w.Header().Set("Content-Disposition", `attachment; filename="drywire-identity.json"`)
	writeJSON(w, http.StatusOK, e)
}

// deleteMe: DELETE /v1/me. The identity and its memberships are deleted
// (through the deletion ledger); each space it owned goes to its oldest
// admin or member, or is deleted if nobody else is in it. Its session,
// event streams and tokens end at once.
func (s *Server) deleteMe(w http.ResponseWriter, r *http.Request, c caller) {
	id, ok := c.registered(w)
	if !ok {
		return
	}
	d, err := s.store.DeleteIdentity(r.Context(), id.ID)
	if err != nil {
		s.fail(w, err)
		return
	}
	s.identityGone(c.key, d)
	w.WriteHeader(http.StatusNoContent)
}

// identityGone ends everything live that belonged to a deleted identity.
func (s *Server) identityGone(key store.Key, d store.Deleted) {
	if p, ok := s.presence.Of(key); ok {
		s.endSession(p.Session, presence.Leave)
	}
	s.presence.Kick(nil, key)
	for _, space := range d.Spaces {
		s.spaceGone(space)
	}
	s.auth.Revoke(key)
}
