// Package relay is Drywire's data plane: it receives UDP packets and copies
// each audio packet to the other members of the sender's room, without
// decrypting anything (docs/protocol.md, section 10).
//
// The logic lives in Handle, which takes one datagram and returns the
// datagrams to send. It does no I/O, so tests can drive it with a fake clock.
// Serve is the thin loop that connects Handle to a UDP socket.
//
// Sessions come from the control plane, which adds one when a member joins
// a room (AddSession), removes it when they leave (RemoveSession), and
// calls Expire every second to learn which sessions went silent.
package relay

import (
	"errors"
	"log/slog"
	"maps"
	"net"
	"net/netip"
	"slices"
	"sync"
	"time"

	"github.com/lignenative/drywire/server/internal/ratelimit"
	"github.com/lignenative/drywire/server/internal/wire"
)

// Limits of protocol section 10.3.
const (
	SessionPacketRate  = 600
	SessionPacketBurst = 200
	SessionByteRate    = 128 * 1024
	SessionByteBurst   = 32 * 1024
	IPPacketRate       = 3000
	IPPacketBurst      = 600
	SessionTimeout     = 30 * time.Second
)

const (
	// maxTrackedIPs bounds the memory of the per-IP limiter. Beyond it, new
	// addresses share one bucket until idle entries are swept, so a flood
	// from many forged addresses cannot exhaust memory.
	maxTrackedIPs = 4096
	sweepInterval = time.Second
)

// Reasons for dropping a datagram, besides the wire error codes. They show
// up in the status counters.
const (
	DropIPRateLimited      = "ip_rate_limited"
	DropSessionRateLimited = "session_rate_limited"
	DropUnknownSession     = "unknown_session"
	DropNotBound           = "not_bound"
	DropStaleCounter       = "stale_counter"
	DropDuplicate          = "duplicate"
	DropRelayOnlyType      = "relay_only_type"
)

// Datagram is a packet to send. Data may point into the buffer that was
// passed to Handle, so it must be sent before that buffer is reused.
type Datagram struct {
	To   netip.AddrPort
	Data []byte
}

// Stats is a snapshot of the relay's counters. It contains no address and
// no session number.
type Stats struct {
	Rooms         int               `json:"rooms"`
	Sessions      int               `json:"sessions"`
	BoundSessions int               `json:"bound_sessions"`
	Received      uint64            `json:"packets_received"`
	Sent          uint64            `json:"packets_sent"`
	Dropped       map[string]uint64 `json:"packets_dropped"`
}

// Errors of AddSession.
var (
	ErrSessionExists = errors.New("session number already in use")
	ErrRoomFull      = errors.New("the room is full")
	ErrBadSession    = errors.New("session 0 or a secret of the wrong size")
)

// Relay holds every room and session. All state is guarded by mu; Handle
// holds it for the few microseconds a packet takes.
type Relay struct {
	mu        sync.Mutex
	now       func() time.Time
	log       *slog.Logger
	rooms     map[string]*room
	sessions  map[uint32]*session
	ips       map[netip.Addr]*ratelimit.Bucket
	overflow  ratelimit.Bucket // shared by new IPs while ips is full
	lastSweep time.Time
	stats     Stats
}

type room struct {
	name    string
	members []*session
}

type session struct {
	id     uint32
	secret []byte
	room   *room
	// static sessions come from a phase 1 rooms file: they never expire,
	// only unbind.
	static bool

	// Binding: the address that last proved it holds the secret.
	bound       bool
	addr        netip.AddrPort
	lastSeen    time.Time
	lastCounter uint64 // highest hello counter accepted, kept after expiry

	packets ratelimit.Bucket
	bytes   ratelimit.Bucket
	uplink  uplink
}

// New builds a relay with no session. now is the clock; pass time.Now
// outside tests.
func New(now func() time.Time, log *slog.Logger) *Relay {
	return &Relay{
		now:      now,
		log:      log,
		rooms:    map[string]*room{},
		sessions: map[uint32]*session{},
		ips:      map[netip.Addr]*ratelimit.Bucket{},
		overflow: ratelimit.New(IPPacketRate, IPPacketBurst, now()),
		stats:    Stats{Dropped: map[string]uint64{}},
	}
}

// AddSession creates session id in the named room, creating the room if
// needed. Its client must say hello within SessionTimeout, or Expire
// removes it.
func (r *Relay) AddSession(roomName string, id uint32, secret []byte) error {
	return r.addSession(roomName, id, secret, false)
}

func (r *Relay) addSession(roomName string, id uint32, secret []byte, static bool) error {
	if id == 0 || len(secret) != wire.SecretSize {
		return ErrBadSession
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.sessions[id] != nil {
		return ErrSessionExists
	}
	rm := r.rooms[roomName]
	if rm == nil {
		rm = &room{name: roomName}
		r.rooms[roomName] = rm
	}
	if len(rm.members) >= MaxMembersPerRoom {
		return ErrRoomFull
	}
	now := r.now()
	s := &session{
		id:       id,
		secret:   slices.Clone(secret),
		room:     rm,
		static:   static,
		lastSeen: now,
		packets:  ratelimit.New(SessionPacketRate, SessionPacketBurst, now),
		bytes:    ratelimit.New(SessionByteRate, SessionByteBurst, now),
	}
	rm.members = append(rm.members, s)
	r.sessions[id] = s
	return nil
}

// RemoveSession forgets a session; its packets are dropped from now on. It
// reports whether the session existed.
func (r *Relay) RemoveSession(id uint32) bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.remove(id)
}

func (r *Relay) remove(id uint32) bool {
	s := r.sessions[id]
	if s == nil {
		return false
	}
	delete(r.sessions, id)
	s.room.members = slices.DeleteFunc(s.room.members, func(m *session) bool { return m == s })
	if len(s.room.members) == 0 {
		delete(r.rooms, s.room.name)
	}
	return true
}

// Expire removes the sessions that sent nothing for SessionTimeout, bound
// or never bound, and returns their numbers: for the control plane, these
// members lost their connection. Static sessions only unbind (in Handle).
func (r *Relay) Expire() []uint32 {
	r.mu.Lock()
	defer r.mu.Unlock()
	now := r.now()
	var gone []uint32
	for id, s := range r.sessions {
		if !s.static && now.Sub(s.lastSeen) > SessionTimeout {
			gone = append(gone, id)
		}
	}
	for _, id := range gone {
		r.remove(id)
	}
	slices.Sort(gone)
	return gone
}

// Serve reads datagrams from conn until it is closed.
func (r *Relay) Serve(conn *net.UDPConn) error {
	// One byte more than the largest valid packet, so that an oversized
	// datagram is seen as too large rather than silently truncated.
	buf := make([]byte, wire.MaxPacketSize+1)
	for {
		n, from, err := conn.ReadFromUDPAddrPort(buf)
		if errors.Is(err, net.ErrClosed) {
			return nil
		}
		if err != nil {
			r.log.Warn("udp read failed", "error", err)
			continue
		}
		for _, d := range r.Handle(buf[:n], from) {
			if _, err := conn.WriteToUDPAddrPort(d.Data, d.To); err != nil {
				r.log.Debug("udp write failed", "error", err)
			}
		}
	}
}

// Handle processes one datagram received from the given address and returns
// the datagrams to send in response. It follows protocol section 10.1 step
// by step.
func (r *Relay) Handle(packet []byte, from netip.AddrPort) []Datagram {
	r.mu.Lock()
	defer r.mu.Unlock()

	now := r.now()
	r.sweep(now)
	r.stats.Received++

	// 1. Per-IP rate limit, before spending any work on the packet.
	if !r.allowIP(from.Addr().Unmap(), now) {
		return r.drop(DropIPRateLimited)
	}

	// 2. Header and size.
	h, err := wire.Parse(packet)
	if err != nil {
		return r.drop(err.Error())
	}
	s := r.sessions[h.Session]
	if s == nil {
		return r.drop(DropUnknownSession)
	}

	// 3. A hello may come from any address: it is how an address gets bound.
	if h.Type == wire.TypeHello {
		return r.hello(s, packet, from, now)
	}

	// 4. Everything else must come from the bound address.
	if !s.bound || s.addr != from {
		return r.drop(DropNotBound)
	}

	// 5. Per-session limits.
	if !s.packets.Allow(now, 1) || !s.bytes.Allow(now, float64(len(packet))) {
		return r.drop(DropSessionRateLimited)
	}
	s.lastSeen = now

	// 6. By type.
	switch h.Type {
	case wire.TypeKeepalive:
		return nil
	case wire.TypePing:
		pong := wire.NewPong(s.id, wire.Pong{
			Number:        h.Sequence,
			ClientTime:    wire.PingClientTime(packet),
			AudioReceived: s.uplink.receivedCount(),
			AudioLost:     s.uplink.lostCount(),
		})
		return r.send(Datagram{To: s.addr, Data: pong})
	case wire.TypeAudio:
		if s.uplink.record(h.KeyEpoch, h.Sequence) {
			return r.drop(DropDuplicate)
		}
		return r.send(r.toRoom(s, packet)...)
	case wire.TypeAnnounce:
		// Signed by the sender for the other members; the relay cannot and
		// does not verify it (section 6.7).
		return r.send(r.toRoom(s, packet)...)
	default: // hello_ack and pong are only ever sent by the relay
		return r.drop(DropRelayOnlyType)
	}
}

// toRoom addresses an unchanged copy of packet to every other bound member
// of the sender's room.
func (r *Relay) toRoom(s *session, packet []byte) []Datagram {
	var out []Datagram
	for _, m := range s.room.members {
		if m != s && m.bound {
			out = append(out, Datagram{To: m.addr, Data: packet})
		}
	}
	return out
}

// hello verifies a hello and, if it is valid and newer than the last one,
// binds the session to the sender's address (protocol section 6.1).
func (r *Relay) hello(s *session, packet []byte, from netip.AddrPort, now time.Time) []Datagram {
	counter, err := wire.VerifyHandshake(packet, s.secret)
	if err != nil {
		return r.drop(err.Error())
	}
	if counter <= s.lastCounter {
		return r.drop(DropStaleCounter)
	}
	if !s.bound || s.addr != from {
		r.log.Info("session bound", "session", s.id, "room", s.room.name, "moved", s.bound)
	}
	s.lastCounter = counter
	s.bound, s.addr, s.lastSeen = true, from, now
	s.uplink = uplink{} // counters restart at every accepted hello (section 10.4)
	return r.send(Datagram{To: from, Data: wire.NewHelloAck(s.id, counter, s.secret)})
}

func (r *Relay) allowIP(ip netip.Addr, now time.Time) bool {
	b, ok := r.ips[ip]
	if !ok {
		if len(r.ips) >= maxTrackedIPs {
			return r.overflow.Allow(now, 1)
		}
		fresh := ratelimit.New(IPPacketRate, IPPacketBurst, now)
		b = &fresh
		r.ips[ip] = b
	}
	return b.Allow(now, 1)
}

// sweep forgets idle IP buckets and unbinds sessions that went silent. It
// runs at most once per sweepInterval, from Handle and Stats, so the relay
// needs no background goroutine.
func (r *Relay) sweep(now time.Time) {
	if now.Sub(r.lastSweep) < sweepInterval {
		return
	}
	r.lastSweep = now
	for ip, b := range r.ips {
		if b.Full(now) {
			delete(r.ips, ip)
		}
	}
	for _, s := range r.sessions {
		if s.bound && now.Sub(s.lastSeen) > SessionTimeout {
			s.bound, s.addr = false, netip.AddrPort{}
			r.log.Info("session timed out", "session", s.id, "room", s.room.name)
		}
	}
}

func (r *Relay) drop(reason string) []Datagram {
	r.stats.Dropped[reason]++
	return nil
}

func (r *Relay) send(out ...Datagram) []Datagram {
	r.stats.Sent += uint64(len(out))
	return out
}

// Stats returns a copy of the counters.
func (r *Relay) Stats() Stats {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.sweep(r.now())

	st := r.stats
	st.Rooms = len(r.rooms)
	st.Sessions = len(r.sessions)
	st.Dropped = maps.Clone(r.stats.Dropped)
	for _, s := range r.sessions {
		if s.bound {
			st.BoundSessions++
		}
	}
	return st
}
