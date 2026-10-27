package api

import "github.com/lignenative/drywire/server/internal/store"

// These hooks end what a change in the store leaves behind. Until the
// control plane hands out UDP sessions, there is nothing to end.

// spaceGone is called once a space is deleted.
func (s *Server) spaceGone(store.ID) {}

// roomGone is called once a room is deleted.
func (s *Server) roomGone(store.Room) {}

// memberGone is called once the holder of key is out of the space.
func (s *Server) memberGone(store.ID, store.Key) {}
