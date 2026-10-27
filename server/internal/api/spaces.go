package api

import (
	"crypto/rand"
	"encoding/hex"
	"net/http"
	"slices"
	"time"

	"github.com/lignenative/drywire/server/internal/store"
)

// SpaceView is a space as one of its members sees it.
type SpaceView struct {
	Space   store.Space    `json:"space"`
	Role    store.Role     `json:"role"`
	Rooms   []store.Room   `json:"rooms"`
	Members []store.Member `json:"members,omitempty"`
}

func (s *Server) spaceView(r *http.Request, space store.ID, role store.Role, withMembers bool) (SpaceView, error) {
	v := SpaceView{Role: role}
	var err error
	if v.Space, err = s.store.Space(r.Context(), space); err != nil {
		return v, err
	}
	if v.Rooms, err = s.store.Rooms(r.Context(), space); err != nil {
		return v, err
	}
	if withMembers {
		v.Members, err = s.store.Members(r.Context(), space)
	}
	return v, err
}

// listSpaces: GET /v1/spaces, the caller's spaces.
func (s *Server) listSpaces(w http.ResponseWriter, r *http.Request, c caller) {
	if c.identity == nil {
		writeJSON(w, http.StatusOK, []store.Membership{})
		return
	}
	ms, err := s.store.Memberships(r.Context(), c.identity.ID)
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusOK, ms)
}

type createSpaceRequest struct {
	Name        string `json:"name"`
	Room        string `json:"room"`
	DisplayName string `json:"display_name"`
}

// createSpace: POST /v1/spaces. Only the identities the operator lists may
// create spaces (DRYWIRE_SPACE_CREATORS): an instance is not an open
// sign-up (spec section 9).
func (s *Server) createSpace(w http.ResponseWriter, r *http.Request, c caller) {
	if !s.cfg.AnyoneCreates && !slices.Contains(s.cfg.SpaceCreators, c.key) {
		writeError(w, http.StatusForbidden, "this instance does not let you create spaces: ask its operator")
		return
	}
	var req createSpaceRequest
	if !readJSON(w, r, &req) {
		return
	}
	if req.Room == "" {
		req.Room = "general"
	}
	space, _, err := s.store.CreateSpace(r.Context(), c.key, req.DisplayName, req.Name, req.Room, s.cfg.MaxMembers)
	if err != nil {
		s.fail(w, err)
		return
	}
	v, err := s.spaceView(r, space.ID, store.RoleOwner, true)
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusCreated, v)
}

// getSpace: GET /v1/spaces/{space}, with its rooms and members.
func (s *Server) getSpace(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	role, ok := s.role(w, r, c, space, store.RoleMember)
	if !ok {
		return
	}
	v, err := s.spaceView(r, space, role, true)
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusOK, v)
}

// deleteSpace: DELETE /v1/spaces/{space}, by its owner.
func (s *Server) deleteSpace(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if _, ok := s.role(w, r, c, space, store.RoleOwner); !ok {
		return
	}
	if err := s.store.DeleteSpace(r.Context(), space); err != nil {
		s.fail(w, err)
		return
	}
	s.spaceGone(space)
	w.WriteHeader(http.StatusNoContent)
}

type nameRequest struct {
	Name string `json:"name"`
}

// createRoom: POST /v1/spaces/{space}/rooms, by an admin.
func (s *Server) createRoom(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if _, ok := s.role(w, r, c, space, store.RoleAdmin); !ok {
		return
	}
	var req nameRequest
	if !readJSON(w, r, &req) {
		return
	}
	room, err := s.store.CreateRoom(r.Context(), space, req.Name)
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusCreated, room)
}

// deleteRoom: DELETE /v1/rooms/{room}, by an admin of its space.
func (s *Server) deleteRoom(w http.ResponseWriter, r *http.Request, c caller) {
	id, ok := pathID(w, r, "room")
	if !ok {
		return
	}
	room, err := s.store.Room(r.Context(), id)
	if err != nil {
		s.fail(w, err)
		return
	}
	if _, ok := s.role(w, r, c, room.SpaceID, store.RoleAdmin); !ok {
		return
	}
	if err := s.store.DeleteRoom(r.Context(), id); err != nil {
		s.fail(w, err)
		return
	}
	s.roomGone(room)
	w.WriteHeader(http.StatusNoContent)
}

// target finds the member a request is about, and checks the caller ranks
// above them: an admin acts on members, the owner on everyone.
func (s *Server) target(w http.ResponseWriter, r *http.Request, c caller, space store.ID, callerRole store.Role) (store.Member, bool) {
	key, ok := pathKey(w, r, "key")
	if !ok {
		return store.Member{}, false
	}
	members, err := s.store.Members(r.Context(), space)
	if err != nil {
		s.fail(w, err)
		return store.Member{}, false
	}
	i := slices.IndexFunc(members, func(m store.Member) bool { return m.PublicKey == key })
	if i < 0 {
		writeError(w, http.StatusNotFound, "no such member")
		return store.Member{}, false
	}
	if members[i].Role.Rank() >= callerRole.Rank() {
		writeError(w, http.StatusForbidden, "you can only act on members below your role")
		return store.Member{}, false
	}
	return members[i], true
}

type roleRequest struct {
	Role store.Role `json:"role"`
}

// setRole: PATCH /v1/spaces/{space}/members/{key}, by the owner. Making
// someone the owner hands the space over; the old owner becomes an admin.
func (s *Server) setRole(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if _, ok := s.role(w, r, c, space, store.RoleOwner); !ok {
		return
	}
	m, ok := s.target(w, r, c, space, store.RoleOwner)
	if !ok {
		return
	}
	var req roleRequest
	if !readJSON(w, r, &req) {
		return
	}
	var err error
	switch req.Role {
	case store.RoleOwner:
		err = s.store.TransferOwnership(r.Context(), space, c.identity.ID, m.IdentityID)
	case store.RoleAdmin, store.RoleMember:
		err = s.store.SetRole(r.Context(), space, m.IdentityID, req.Role)
	default:
		writeError(w, http.StatusBadRequest, "role must be owner, admin or member")
		return
	}
	if err != nil {
		s.fail(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

// removeMember: DELETE /v1/spaces/{space}/members/{key}. With the caller's
// own key it leaves the space; otherwise an admin removes a member, or the
// owner anyone. The owner cannot leave: they hand the space over or delete
// it.
func (s *Server) removeMember(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	role, ok := s.role(w, r, c, space, store.RoleMember)
	if !ok {
		return
	}
	if r.PathValue("key") == c.key.String() {
		if role == store.RoleOwner {
			writeError(w, http.StatusConflict, "the owner cannot leave: hand the space to someone else, or delete it")
			return
		}
		if err := s.store.RemoveMember(r.Context(), space, c.identity.ID); err != nil {
			s.fail(w, err)
			return
		}
		s.memberGone(space, c.key)
		w.WriteHeader(http.StatusNoContent)
		return
	}
	if role.Rank() < store.RoleAdmin.Rank() {
		writeError(w, http.StatusForbidden, "this needs the admin role")
		return
	}
	m, ok := s.target(w, r, c, space, role)
	if !ok {
		return
	}
	if err := s.store.RemoveMember(r.Context(), space, m.IdentityID); err != nil {
		s.fail(w, err)
		return
	}
	s.memberGone(space, m.PublicKey)
	w.WriteHeader(http.StatusNoContent)
}

// listBans: GET /v1/spaces/{space}/bans, by an admin.
func (s *Server) listBans(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if _, ok := s.role(w, r, c, space, store.RoleAdmin); !ok {
		return
	}
	keys, err := s.store.Bans(r.Context(), space)
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusOK, keys)
}

type banRequest struct {
	PublicKey store.Key `json:"public_key"`
}

// ban: POST /v1/spaces/{space}/bans, by an admin. A member is removed at
// once; a key that is not a member yet can never redeem an invitation.
func (s *Server) ban(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	role, ok := s.role(w, r, c, space, store.RoleAdmin)
	if !ok {
		return
	}
	var req banRequest
	if !readJSON(w, r, &req) {
		return
	}
	members, err := s.store.Members(r.Context(), space)
	if err != nil {
		s.fail(w, err)
		return
	}
	if i := slices.IndexFunc(members, func(m store.Member) bool { return m.PublicKey == req.PublicKey }); i >= 0 &&
		members[i].Role.Rank() >= role.Rank() {
		writeError(w, http.StatusForbidden, "you can only ban members below your role")
		return
	}
	if err := s.store.Ban(r.Context(), space, req.PublicKey); err != nil {
		s.fail(w, err)
		return
	}
	s.memberGone(space, req.PublicKey)
	w.WriteHeader(http.StatusNoContent)
}

// unban: DELETE /v1/spaces/{space}/bans/{key}, by an admin.
func (s *Server) unban(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if _, ok := s.role(w, r, c, space, store.RoleAdmin); !ok {
		return
	}
	key, ok := pathKey(w, r, "key")
	if !ok {
		return
	}
	if err := s.store.Unban(r.Context(), space, key); err != nil {
		s.fail(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

// listInvitations: GET /v1/spaces/{space}/invitations, by an admin: the
// invitations that can still be used, without their codes.
func (s *Server) listInvitations(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if _, ok := s.role(w, r, c, space, store.RoleAdmin); !ok {
		return
	}
	invs, err := s.store.Invitations(r.Context(), space)
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusOK, invs)
}

type invitationRequest struct {
	ExpiresIn int `json:"expires_in"` // seconds; 0 means the default
	MaxUses   int `json:"max_uses"`   // 0 means the default
}

// createInvitation: POST /v1/spaces/{space}/invitations, by an admin. The
// code is in this answer only; the client puts it in a link with the space
// key, which the server never sees.
func (s *Server) createInvitation(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if _, ok := s.role(w, r, c, space, store.RoleAdmin); !ok {
		return
	}
	var req invitationRequest
	if !readJSON(w, r, &req) {
		return
	}
	lifetime := time.Duration(req.ExpiresIn) * time.Second
	if req.ExpiresIn == 0 {
		lifetime = defaultInviteLifetime
	}
	if req.MaxUses == 0 {
		req.MaxUses = defaultInviteUses
	}
	if lifetime < time.Minute || lifetime > maxInviteLifetime || req.MaxUses < 1 || req.MaxUses > maxInviteUses {
		writeError(w, http.StatusBadRequest, "an invitation lasts from a minute to 30 days, for 1 to 100 uses")
		return
	}
	var code [store.CodeSize]byte
	rand.Read(code[:]) // never fails (crypto/rand documentation)
	inv, err := s.store.CreateInvitation(r.Context(), space, c.identity.ID, code, lifetime, req.MaxUses)
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusCreated, map[string]any{
		"invitation": inv,
		"code":       hex.EncodeToString(code[:]),
		"instance":   s.auth.Audience(),
	})
}

// revokeInvitation: DELETE /v1/spaces/{space}/invitations/{id}, by an
// admin.
func (s *Server) revokeInvitation(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if _, ok := s.role(w, r, c, space, store.RoleAdmin); !ok {
		return
	}
	id, ok := pathID(w, r, "id")
	if !ok {
		return
	}
	if err := s.store.RevokeInvitation(r.Context(), space, id); err != nil {
		s.fail(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

type redeemRequest struct {
	Code        string `json:"code"`
	DisplayName string `json:"display_name"`
}

// redeem: POST /v1/invitations/redeem. The caller joins the space as a
// member; its identity is created with the display name if it is new.
func (s *Server) redeem(w http.ResponseWriter, r *http.Request, c caller) {
	var req redeemRequest
	if !readJSON(w, r, &req) {
		return
	}
	var code [store.CodeSize]byte
	raw, err := hex.DecodeString(req.Code)
	if err != nil || len(raw) != len(code) {
		writeError(w, http.StatusNotFound, store.ErrInvalidCode.Error())
		return
	}
	copy(code[:], raw)
	space, err := s.store.Redeem(r.Context(), code, c.key, req.DisplayName)
	if err != nil {
		s.fail(w, err)
		return
	}
	id, err := s.store.IdentityByKey(r.Context(), c.key)
	if err != nil {
		s.fail(w, err)
		return
	}
	role, err := s.store.Role(r.Context(), space.ID, id.ID)
	if err != nil {
		s.fail(w, err)
		return
	}
	v, err := s.spaceView(r, space.ID, role, true)
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusOK, v)
}
