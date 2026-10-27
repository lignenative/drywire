package relay

import (
	"bytes"
	"encoding/hex"
	"fmt"
	"io"
	"log/slog"
	"net/netip"
	"testing"
	"time"

	"github.com/lignenative/drywire/server/internal/wire"
)

// These tests drive Handle directly with a fake clock. The end-to-end tests
// over real sockets are in serve_test.go.

// fakeClock is a clock that only moves when the test says so.
type fakeClock struct{ t time.Time }

func (c *fakeClock) now() time.Time          { return c.t }
func (c *fakeClock) advance(d time.Duration) { c.t = c.t.Add(d) }

func secretFor(session uint32) []byte {
	return bytes.Repeat([]byte{byte(session)}, wire.SecretSize)
}

// testRooms has two rooms: sessions 1, 2 and 3 in "lobby", 4 in "solo".
func testRooms() RoomsFile {
	member := func(s uint32) MemberConfig {
		return MemberConfig{Session: s, Secret: hex.EncodeToString(secretFor(s))}
	}
	return RoomsFile{Rooms: []RoomConfig{
		{Name: "lobby", Members: []MemberConfig{member(1), member(2), member(3)}},
		{Name: "solo", Members: []MemberConfig{member(4)}},
	}}
}

func newTestRelay(t *testing.T) (*Relay, *fakeClock) {
	t.Helper()
	f := testRooms()
	if err := f.Validate(); err != nil {
		t.Fatal(err)
	}
	clock := &fakeClock{t: time.Unix(1_800_000_000, 0)}
	return New(f, clock.now, slog.New(slog.NewTextHandler(io.Discard, nil))), clock
}

// addr returns a distinct client address per number: 10.0.0.n:5000+n.
func addr(n int) netip.AddrPort {
	return netip.MustParseAddrPort(fmt.Sprintf("10.0.0.%d:%d", n, 5000+n))
}

// bind sends a valid hello for session from a, and checks the answer.
func bind(t *testing.T, r *Relay, session uint32, a netip.AddrPort, counter uint64) {
	t.Helper()
	out := r.Handle(wire.NewHello(session, counter, secretFor(session)), a)
	want := wire.NewHelloAck(session, counter, secretFor(session))
	if len(out) != 1 || out[0].To != a || !bytes.Equal(out[0].Data, want) {
		t.Fatalf("hello from session %d: got %v, want one hello_ack to %v", session, out, a)
	}
}

func audioPacket(session uint32, epoch uint8, seq uint32, size int) []byte {
	b := wire.Header{Version: wire.Version, Type: wire.TypeAudio, KeyEpoch: epoch,
		Session: session, Sequence: seq, Timestamp: seq * 240}.Bytes()
	return append(b, make([]byte, size-wire.HeaderSize)...)
}

func destinations(out []Datagram) []netip.AddrPort {
	var to []netip.AddrPort
	for _, d := range out {
		to = append(to, d.To)
	}
	return to
}

func expectDropped(t *testing.T, r *Relay, reason string, want uint64) {
	t.Helper()
	if got := r.Stats().Dropped[reason]; got != want {
		t.Fatalf("dropped for %s = %d, want %d", reason, got, want)
	}
}

func TestNothingIsSentBeforeAValidHello(t *testing.T) {
	r, _ := newTestRelay(t)
	stranger := addr(9)

	packets := map[string][]byte{
		"ping":                    wire.NewPing(1, 1, 1),
		"keepalive":               wire.NewKeepalive(1),
		"audio":                   audioPacket(1, 0, 0, 40),
		"hello, wrong secret":     wire.NewHello(1, 1, secretFor(2)),
		"hello, unknown room":     wire.NewHello(99, 1, secretFor(99)),
		"hello_ack from a client": wire.NewHelloAck(1, 1, secretFor(1)),
		"garbage":                 []byte("not a drywire packet"),
	}
	for name, p := range packets {
		if out := r.Handle(p, stranger); out != nil {
			t.Errorf("%s: the relay answered an unverified address: %v", name, out)
		}
	}
	if st := r.Stats(); st.Sent != 0 || st.BoundSessions != 0 {
		t.Fatalf("stats after unverified traffic: %+v", st)
	}
}

func TestHelloCounterMustIncrease(t *testing.T) {
	r, _ := newTestRelay(t)
	bind(t, r, 1, addr(1), 100)

	// A captured hello replayed from another address must not steal the
	// session.
	if out := r.Handle(wire.NewHello(1, 100, secretFor(1)), addr(9)); out != nil {
		t.Fatal("a replayed hello was answered")
	}
	if out := r.Handle(wire.NewHello(1, 99, secretFor(1)), addr(9)); out != nil {
		t.Fatal("an older hello was answered")
	}
	expectDropped(t, r, DropStaleCounter, 2)

	// Audio from session 1 still goes out from addr(1) only.
	if out := r.Handle(audioPacket(1, 0, 0, 40), addr(9)); out != nil {
		t.Fatal("audio from the replayer's address was forwarded")
	}
}

func TestAudioReachesTheOtherMembersOfTheRoom(t *testing.T) {
	r, _ := newTestRelay(t)
	for s := uint32(1); s <= 4; s++ {
		bind(t, r, s, addr(int(s)), 1)
	}

	packet := audioPacket(1, 0, 0, 60)
	out := r.Handle(packet, addr(1))

	got := destinations(out)
	if len(got) != 2 || got[0] != addr(2) || got[1] != addr(3) {
		t.Fatalf("audio from session 1 went to %v, want [%v %v]", got, addr(2), addr(3))
	}
	for _, d := range out {
		if !bytes.Equal(d.Data, packet) {
			t.Fatal("the relay changed the packet it forwarded")
		}
	}
}

func TestUnboundMembersReceiveNothing(t *testing.T) {
	r, _ := newTestRelay(t)
	bind(t, r, 1, addr(1), 1)
	bind(t, r, 2, addr(2), 1)

	got := destinations(r.Handle(audioPacket(1, 0, 0, 40), addr(1)))
	if len(got) != 1 || got[0] != addr(2) {
		t.Fatalf("audio went to %v, want only %v (session 3 never said hello)", got, addr(2))
	}
}

func TestHelloMovesTheSession(t *testing.T) {
	r, _ := newTestRelay(t)
	bind(t, r, 1, addr(1), 1)
	bind(t, r, 2, addr(2), 1)

	// Session 2 changes network and says hello from a new address.
	moved := netip.MustParseAddrPort("192.0.2.7:40000")
	bind(t, r, 2, moved, 2)

	got := destinations(r.Handle(audioPacket(1, 0, 0, 40), addr(1)))
	if len(got) != 1 || got[0] != moved {
		t.Fatalf("audio went to %v, want the new address %v", got, moved)
	}
	if out := r.Handle(wire.NewKeepalive(2), addr(2)); out != nil {
		t.Fatal("the old address is still accepted")
	}
	expectDropped(t, r, DropNotBound, 1)
}

func TestPingReportsUplinkLoss(t *testing.T) {
	r, _ := newTestRelay(t)
	bind(t, r, 1, addr(1), 1)

	// Sequences 0..9 with 3 and 7 missing, 5 duplicated, then 3 arrives late.
	for _, seq := range []uint32{0, 1, 2, 4, 5, 5, 6, 8, 9, 3} {
		r.Handle(audioPacket(1, 0, seq, 40), addr(1))
	}
	out := r.Handle(wire.NewPing(1, 42, 777), addr(1))
	if len(out) != 1 || out[0].To != addr(1) {
		t.Fatalf("ping got %v, want one pong", out)
	}
	if len(out[0].Data) > wire.PingSize {
		t.Fatal("the pong is larger than the ping")
	}
	want := wire.Pong{Number: 42, ClientTime: 777, AudioReceived: 9, AudioLost: 1}
	if got := wire.DecodePong(out[0].Data); got != want {
		t.Fatalf("pong = %+v, want %+v", got, want)
	}
	expectDropped(t, r, DropDuplicate, 1)
}

func TestLossCountingAcrossKeyEpochs(t *testing.T) {
	r, _ := newTestRelay(t)
	bind(t, r, 1, addr(1), 1)

	send := func(epoch uint8, seqs ...uint32) {
		for _, s := range seqs {
			r.Handle(audioPacket(1, epoch, s, 40), addr(1))
		}
	}
	send(0, 0, 1, 3)    // epoch 0: sequence 2 lost
	send(1, 0, 1, 2, 4) // epoch 1, the sequence restarts: 3 lost
	send(0, 2)          // late packet of the old epoch: forwarded, not counted

	got := wire.DecodePong(r.Handle(wire.NewPing(1, 1, 0), addr(1))[0].Data)
	if got.AudioReceived != 7 || got.AudioLost != 2 {
		t.Fatalf("received %d, lost %d; want 7 and 2", got.AudioReceived, got.AudioLost)
	}

	// A new hello restarts the counters.
	bind(t, r, 1, addr(1), 2)
	got = wire.DecodePong(r.Handle(wire.NewPing(1, 2, 0), addr(1))[0].Data)
	if got.AudioReceived != 0 || got.AudioLost != 0 {
		t.Fatalf("after hello: received %d, lost %d; want 0 and 0", got.AudioReceived, got.AudioLost)
	}
}

func TestSessionPacketRateLimit(t *testing.T) {
	r, clock := newTestRelay(t)
	bind(t, r, 1, addr(1), 1)
	bind(t, r, 2, addr(2), 1)

	forwarded := 0
	for seq := range uint32(SessionPacketBurst + 50) {
		forwarded += len(r.Handle(audioPacket(1, 0, seq, 40), addr(1)))
	}
	if forwarded != SessionPacketBurst {
		t.Fatalf("forwarded %d packets of a burst, want %d", forwarded, SessionPacketBurst)
	}

	// 100 ms later, 60 more packets are allowed (600 per second).
	clock.advance(100 * time.Millisecond)
	forwarded = 0
	for seq := range uint32(100) {
		forwarded += len(r.Handle(audioPacket(1, 0, 1000+seq, 40), addr(1)))
	}
	if forwarded != SessionPacketRate/10 {
		t.Fatalf("forwarded %d packets after 100 ms, want %d", forwarded, SessionPacketRate/10)
	}
}

func TestSessionByteRateLimit(t *testing.T) {
	r, _ := newTestRelay(t)
	bind(t, r, 1, addr(1), 1)
	bind(t, r, 2, addr(2), 1)

	forwarded := 0
	for seq := range uint32(50) {
		forwarded += len(r.Handle(audioPacket(1, 0, seq, wire.MaxPacketSize), addr(1)))
	}
	if want := SessionByteBurst / wire.MaxPacketSize; forwarded != want {
		t.Fatalf("forwarded %d full-size packets, want %d", forwarded, want)
	}
}

func TestIPRateLimit(t *testing.T) {
	r, _ := newTestRelay(t)

	// Any traffic counts against the source IP, even invalid packets.
	for range IPPacketBurst + 10 {
		r.Handle([]byte("x"), addr(9))
	}
	expectDropped(t, r, DropIPRateLimited, 10)

	// Another IP is not affected.
	bind(t, r, 1, addr(1), 1)
}

func TestSilentSessionsExpire(t *testing.T) {
	r, clock := newTestRelay(t)
	bind(t, r, 1, addr(1), 1)
	bind(t, r, 2, addr(2), 1)

	clock.advance(SessionTimeout / 2)
	r.Handle(wire.NewKeepalive(1), addr(1))
	clock.advance(SessionTimeout/2 + time.Second)

	if st := r.Stats(); st.BoundSessions != 1 {
		t.Fatalf("bound sessions = %d, want 1 (session 2 went silent)", st.BoundSessions)
	}
	if got := destinations(r.Handle(audioPacket(1, 0, 0, 40), addr(1))); len(got) != 0 {
		t.Fatalf("audio went to %v after session 2 expired", got)
	}

	// The expired session comes back with a new hello, never an old one.
	if out := r.Handle(wire.NewHello(2, 1, secretFor(2)), addr(2)); out != nil {
		t.Fatal("an expired session accepted its old hello counter")
	}
	bind(t, r, 2, addr(2), 2)
}

func TestRelayOnlyTypesAreDropped(t *testing.T) {
	r, _ := newTestRelay(t)
	bind(t, r, 1, addr(1), 1)
	bind(t, r, 2, addr(2), 1)

	pong := wire.NewPong(1, wire.Pong{})
	if out := r.Handle(pong, addr(1)); out != nil {
		t.Fatal("a pong from a client was answered or forwarded")
	}
	expectDropped(t, r, DropRelayOnlyType, 1)
}

func TestValidate(t *testing.T) {
	ok := MemberConfig{Session: 1, Secret: hex.EncodeToString(secretFor(1))}
	tests := map[string]RoomsFile{
		"room without a name": {Rooms: []RoomConfig{{Members: []MemberConfig{ok}}}},
		"duplicate room":      {Rooms: []RoomConfig{{Name: "a"}, {Name: "a"}}},
		"session 0":           {Rooms: []RoomConfig{{Name: "a", Members: []MemberConfig{{Secret: ok.Secret}}}}},
		"duplicate session": {Rooms: []RoomConfig{
			{Name: "a", Members: []MemberConfig{ok}}, {Name: "b", Members: []MemberConfig{ok}}}},
		"short secret": {Rooms: []RoomConfig{{Name: "a", Members: []MemberConfig{{Session: 1, Secret: "abcd"}}}}},
	}
	for name, f := range tests {
		if err := f.Validate(); err == nil {
			t.Errorf("%s: Validate() accepted it", name)
		}
	}
	if err := testRooms().Validate(); err != nil {
		t.Fatalf("valid rooms rejected: %v", err)
	}
}
