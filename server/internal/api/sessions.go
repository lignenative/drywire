package api

import (
	"crypto/rand"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"time"

	"github.com/lignenative/drywire/server/internal/presence"
	"github.com/lignenative/drywire/server/internal/relay"
	"github.com/lignenative/drywire/server/internal/store"
	"github.com/lignenative/drywire/server/internal/wire"
)

// keepaliveEvery is how often an idle event stream gets a comment line, so
// that proxies and NATs keep the connection open.
const keepaliveEvery = 15 * time.Second

// JoinAnswer is what a member gets to enter a room (protocol section
// 13.6): its UDP session, the relay to send to, and who is in the room.
type JoinAnswer struct {
	Session       uint32             `json:"session"`
	SessionSecret string             `json:"session_secret"`
	Relay         string             `json:"relay"`
	Room          store.Room         `json:"room"`
	Sessions      []presence.Session `json:"sessions"`
}

// joinRoom: POST /v1/rooms/{room}/join. The relay gets a new session with
// a fresh random number and secret; the identity's earlier session, in any
// room, ends.
func (s *Server) joinRoom(w http.ResponseWriter, r *http.Request, c caller) {
	id, ok := pathID(w, r, "room")
	if !ok {
		return
	}
	room, err := s.store.Room(r.Context(), id)
	if err != nil {
		s.fail(w, err)
		return
	}
	if _, ok := s.role(w, r, c, room.SpaceID, store.RoleMember); !ok {
		return
	}

	secret := make([]byte, wire.SecretSize)
	rand.Read(secret) // never fails (crypto/rand documentation)
	var session uint32
	for {
		var b [4]byte
		rand.Read(b[:])
		session = binary.BigEndian.Uint32(b[:])
		if session == 0 {
			continue
		}
		err = s.relay.AddSession(room.ID.String(), session, secret)
		if !errors.Is(err, relay.ErrSessionExists) {
			break
		}
	}
	if errors.Is(err, relay.ErrRoomFull) {
		writeError(w, http.StatusConflict, "the room is full")
		return
	}
	if err != nil {
		s.fail(w, err)
		return
	}
	if old, had := s.presence.Add(presence.Session{
		Session: session, Room: room.ID, PublicKey: c.key, Name: c.identity.Name, Space: room.SpaceID,
	}); had {
		s.relay.RemoveSession(old.Session)
	}
	// A removal from the space that raced with this join would have
	// missed the new session: check again, now that it is visible.
	if _, err := s.store.Role(r.Context(), room.SpaceID, c.identity.ID); err != nil {
		s.endSession(session, presence.Leave)
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusOK, JoinAnswer{
		Session:       session,
		SessionSecret: hex.EncodeToString(secret),
		Relay:         s.cfg.RelayAddr,
		Room:          room,
		Sessions:      s.presence.Find(func(p presence.Session) bool { return p.Room == room.ID }),
	})
}

// leaveRoom: POST /v1/leave. The caller's session ends, wherever it is.
func (s *Server) leaveRoom(w http.ResponseWriter, r *http.Request, c caller) {
	if p, ok := s.presence.Of(c.key); ok {
		s.endSession(p.Session, presence.Leave)
	}
	w.WriteHeader(http.StatusNoContent)
}

func (s *Server) endSession(session uint32, why string) {
	s.relay.RemoveSession(session)
	s.presence.Remove(session, why)
}

// ExpireSessions ends the sessions the relay stopped hearing from, as
// disconnections. The server calls it every second.
func (s *Server) ExpireSessions() {
	for _, session := range s.relay.Expire() {
		s.presence.Remove(session, presence.Disconnect)
	}
}

// CloseEvents ends every event stream, so that the HTTP server can stop.
func (s *Server) CloseEvents() { s.presence.Close() }

// events: GET /v1/spaces/{space}/events, Server-Sent Events (protocol
// section 13.6): a snapshot of who is where, then every join, leave and
// disconnect in the space.
func (s *Server) events(w http.ResponseWriter, r *http.Request, c caller) {
	space, ok := pathID(w, r, "space")
	if !ok {
		return
	}
	if _, ok := s.role(w, r, c, space, store.RoleMember); !ok {
		return
	}
	flusher, ok := w.(http.Flusher)
	if !ok {
		s.fail(w, errors.New("the response writer cannot stream"))
		return
	}
	sub, snapshot, err := s.presence.Subscribe(space, c.key)
	if err != nil {
		writeError(w, http.StatusTooManyRequests, err.Error())
		return
	}
	defer s.presence.Unsubscribe(sub)

	// The server's read timeout guards request bodies; a stream that
	// lasts hours must not inherit it.
	http.NewResponseController(w).SetReadDeadline(time.Time{})
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-store")
	// Tells nginx-style proxies not to hold events back in a buffer.
	w.Header().Set("X-Accel-Buffering", "no")
	w.WriteHeader(http.StatusOK)
	send := func(event string, data any) bool {
		raw, _ := json.Marshal(data)
		_, err := fmt.Fprintf(w, "event: %s\ndata: %s\n\n", event, raw)
		flusher.Flush()
		return err == nil
	}
	if !send("snapshot", map[string]any{"sessions": snapshot}) {
		return
	}
	keepalive := time.NewTicker(keepaliveEvery)
	defer keepalive.Stop()
	for {
		select {
		case e := <-sub.Events():
			if !send(e.Type, e) {
				return
			}
		case <-keepalive.C:
			if _, err := fmt.Fprint(w, ": keepalive\n\n"); err != nil {
				return
			}
			flusher.Flush()
		case <-sub.Done():
			return
		case <-r.Context().Done():
			return
		}
	}
}

// spaceGone is called once a space is deleted: its sessions end and its
// event streams close.
func (s *Server) spaceGone(space store.ID) {
	for _, p := range s.presence.Find(func(p presence.Session) bool { return p.Space == space }) {
		s.endSession(p.Session, presence.Leave)
	}
	s.presence.CloseSpace(space)
}

// roomGone is called once a room is deleted: everyone in it leaves.
func (s *Server) roomGone(room store.Room) {
	for _, p := range s.presence.Find(func(p presence.Session) bool { return p.Room == room.ID }) {
		s.endSession(p.Session, presence.Leave)
	}
}

// memberGone is called once the holder of key is out of the space: their
// session there ends and their event streams of that space close.
func (s *Server) memberGone(space store.ID, key store.Key) {
	if p, ok := s.presence.Of(key); ok && p.Space == space {
		s.endSession(p.Session, presence.Leave)
	}
	s.presence.Kick(&space, key)
}
