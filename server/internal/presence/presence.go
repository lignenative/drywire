// Package presence knows who is in which room right now, and tells the
// members of a space when that changes (spec section 10, "Presence").
//
// Presence lives in memory only and is never stored: when the server
// restarts, everyone joins again. Each identity holds at most one session
// on the instance: joining a room ends the session it had elsewhere.
//
// The hub does no I/O. The api package adds and removes sessions, and turns
// subscriptions into Server-Sent Events.
package presence

import (
	"sync"

	"github.com/lignenative/drywire/server/internal/store"
)

// Event types.
const (
	// Join: someone entered a room.
	Join = "join"
	// Leave: someone left on purpose, was removed, or moved to another
	// room.
	Leave = "leave"
	// Disconnect: the relay stopped hearing from someone (protocol
	// section 10.3, session timeout).
	Disconnect = "disconnect"
	// MLS: the space's MLS group moved to a new epoch, or a device waits
	// to be added to it (ADR 0052); only Epoch is set.
	MLS = "mls"
)

const (
	// eventQueue is how many events wait for a slow subscriber before it
	// is dropped; its client reconnects and gets a fresh snapshot.
	eventQueue = 256
	// MaxSubscriptions bounds the event streams one identity holds open.
	// One more ends its oldest (ADR 0047).
	MaxSubscriptions = 8
)

// Session is one member in one room.
type Session struct {
	Session   uint32    `json:"session"`
	Room      store.ID  `json:"room_id"`
	PublicKey store.Key `json:"public_key"`
	Name      string    `json:"name"`
	Space     store.ID  `json:"-"`
}

// Event is a change, as subscribers receive it. Name is empty for leave
// and disconnect.
type Event struct {
	Type      string    `json:"-"`
	Room      store.ID  `json:"room_id"`
	Session   uint32    `json:"session"`
	PublicKey store.Key `json:"public_key"`
	Name      string    `json:"name,omitempty"`
	// Epoch is the group's epoch, for an MLS event.
	Epoch uint64 `json:"-"`
}

// Subscription receives the events of one space.
type Subscription struct {
	space store.ID
	key   store.Key
	// opened orders an identity's subscriptions, oldest first.
	opened uint64
	events chan Event
	done   chan struct{}
	closed bool
}

// Events delivers the space's events in order.
func (s *Subscription) Events() <-chan Event { return s.events }

// Done is closed when the subscription ends from the server's side: the
// subscriber left the space, fell too far behind, opened too many newer
// ones, or the server stops.
func (s *Subscription) Done() <-chan struct{} { return s.done }

// Hub holds the live sessions and the subscriptions. It is safe for
// concurrent use.
type Hub struct {
	mu       sync.Mutex
	sessions map[uint32]Session
	byKey    map[store.Key]uint32
	subs     map[*Subscription]struct{}
	// opened counts the subscriptions ever made, to tell the oldest.
	opened uint64
}

// NewHub returns an empty hub.
func NewHub() *Hub {
	return &Hub{
		sessions: map[uint32]Session{},
		byKey:    map[store.Key]uint32{},
		subs:     map[*Subscription]struct{}{},
	}
}

// Add records a session and announces it. If the identity already had a
// session, that one ends first (announced as a leave) and is returned, so
// the caller can take it off the relay.
func (h *Hub) Add(s Session) (replaced Session, hadOne bool) {
	h.mu.Lock()
	defer h.mu.Unlock()
	if old, ok := h.byKey[s.PublicKey]; ok {
		replaced, hadOne = h.remove(old, Leave)
	}
	h.sessions[s.Session] = s
	h.byKey[s.PublicKey] = s.Session
	h.publish(s.Space, Event{Type: Join, Room: s.Room, Session: s.Session, PublicKey: s.PublicKey, Name: s.Name})
	return replaced, hadOne
}

// Remove ends a session and announces it as a Leave or a Disconnect.
func (h *Hub) Remove(session uint32, why string) (Session, bool) {
	h.mu.Lock()
	defer h.mu.Unlock()
	return h.remove(session, why)
}

func (h *Hub) remove(session uint32, why string) (Session, bool) {
	s, ok := h.sessions[session]
	if !ok {
		return s, false
	}
	delete(h.sessions, session)
	delete(h.byKey, s.PublicKey)
	h.publish(s.Space, Event{Type: why, Room: s.Room, Session: s.Session, PublicKey: s.PublicKey})
	return s, true
}

// Of returns the identity's session, if it has one.
func (h *Hub) Of(key store.Key) (Session, bool) {
	h.mu.Lock()
	defer h.mu.Unlock()
	s, ok := h.sessions[h.byKey[key]]
	return s, ok
}

// Find returns the sessions for which match is true.
func (h *Hub) Find(match func(Session) bool) []Session {
	h.mu.Lock()
	defer h.mu.Unlock()
	var out []Session
	for _, s := range h.sessions {
		if match(s) {
			out = append(out, s)
		}
	}
	return out
}

// Subscribe starts a subscription to the space's events for the holder of
// key, and returns it with the sessions already in the space. Nothing
// happens between the snapshot and the first event.
//
// An identity holds at most MaxSubscriptions: one more ends its oldest,
// as a fifth login ends the oldest token. A client whose network changed
// opens new streams while the server still holds the old ones, which
// nobody will ever read; refusing the new ones would lock it out until
// TCP gave up on the old (ADR 0047).
func (h *Hub) Subscribe(space store.ID, key store.Key) (*Subscription, []Session) {
	h.mu.Lock()
	defer h.mu.Unlock()
	var mine []*Subscription
	for s := range h.subs {
		if s.key == key {
			mine = append(mine, s)
		}
	}
	for len(mine) >= MaxSubscriptions {
		oldest := 0
		for i, s := range mine {
			if s.opened < mine[oldest].opened {
				oldest = i
			}
		}
		h.end(mine[oldest])
		mine = append(mine[:oldest], mine[oldest+1:]...)
	}
	h.opened++
	sub := &Subscription{
		space:  space,
		key:    key,
		opened: h.opened,
		events: make(chan Event, eventQueue),
		done:   make(chan struct{}),
	}
	h.subs[sub] = struct{}{}
	snapshot := []Session{}
	for _, s := range h.sessions {
		if s.Space == space {
			snapshot = append(snapshot, s)
		}
	}
	return sub, snapshot
}

// Unsubscribe ends a subscription from the subscriber's side.
func (h *Hub) Unsubscribe(sub *Subscription) {
	h.mu.Lock()
	defer h.mu.Unlock()
	h.end(sub)
}

// Kick ends the subscriptions of key to the space (every space if space is
// nil): they are no longer a member.
func (h *Hub) Kick(space *store.ID, key store.Key) {
	h.mu.Lock()
	defer h.mu.Unlock()
	for sub := range h.subs {
		if sub.key == key && (space == nil || sub.space == *space) {
			h.end(sub)
		}
	}
}

// PublishMLS tells the space's subscribers that its MLS group changed:
// a commit started `epoch`, or a device waits to join. They fetch what is
// new from the delivery service.
func (h *Hub) PublishMLS(space store.ID, epoch uint64) {
	h.mu.Lock()
	defer h.mu.Unlock()
	h.publish(space, Event{Type: MLS, Epoch: epoch})
}

// CloseSpace ends every subscription to a deleted space.
func (h *Hub) CloseSpace(space store.ID) {
	h.mu.Lock()
	defer h.mu.Unlock()
	for sub := range h.subs {
		if sub.space == space {
			h.end(sub)
		}
	}
}

// Close ends every subscription, when the server stops.
func (h *Hub) Close() {
	h.mu.Lock()
	defer h.mu.Unlock()
	for sub := range h.subs {
		h.end(sub)
	}
}

func (h *Hub) end(sub *Subscription) {
	if !sub.closed {
		sub.closed = true
		close(sub.done)
		delete(h.subs, sub)
	}
}

// publish queues an event for every subscriber of the space. A subscriber
// whose queue is full is dropped rather than waited for.
func (h *Hub) publish(space store.ID, e Event) {
	for sub := range h.subs {
		if sub.space != space {
			continue
		}
		select {
		case sub.events <- e:
		default:
			h.end(sub)
		}
	}
}
