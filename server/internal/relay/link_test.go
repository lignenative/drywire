package relay

import (
	"bytes"
	"testing"

	"github.com/lignenative/drywire/server/internal/wire"
)

// A browser's session is bound to its WebTransport session, a Link, rather
// than to a UDP address (ADR 0038). These tests drive HandleLink with a
// fake link; the real WebTransport server is tested in internal/web.

type fakeLink struct{ name string }

func (*fakeLink) SendDatagram([]byte) error { return nil }

func bindLink(t *testing.T, r *Relay, session uint32, l *fakeLink, n int, counter uint64) {
	t.Helper()
	out := r.HandleLink(wire.NewHello(session, counter, secretFor(session)), addr(n), l)
	want := wire.NewHelloAck(session, counter, secretFor(session))
	if len(out) != 1 || out[0].Link != l || !bytes.Equal(out[0].Data, want) {
		t.Fatalf("hello on link %s: got %v, want one hello_ack through it", l.name, out)
	}
}

func TestLinkAndUDPMembersHearEachOther(t *testing.T) {
	r, _ := newTestRelay(t)
	browser := &fakeLink{name: "browser"}
	bind(t, r, 1, addr(1), 1)
	bindLink(t, r, 2, browser, 2, 1)

	out := r.Handle(audioPacket(1, 0, 0, 40), addr(1))
	if len(out) != 1 || out[0].Link != browser {
		t.Fatalf("UDP audio went to %v, want the browser's link", out)
	}
	out = r.HandleLink(audioPacket(2, 0, 0, 40), addr(2), browser)
	if len(out) != 1 || out[0].Link != nil || out[0].To != addr(1) {
		t.Fatalf("browser audio went to %v, want UDP %v", out, addr(1))
	}
	pong := r.HandleLink(wire.NewPing(2, 1, 0), addr(2), browser)
	if len(pong) != 1 || pong[0].Link != browser {
		t.Fatalf("ping answered with %v, want a pong through the link", pong)
	}
}

func TestALinkIsTheClientWhateverItsAddress(t *testing.T) {
	r, _ := newTestRelay(t)
	browser := &fakeLink{name: "browser"}
	bindLink(t, r, 2, browser, 2, 1)

	// UDP from the link's own address is someone else.
	if out := r.Handle(wire.NewKeepalive(2), addr(2)); out != nil {
		t.Fatal("UDP from the link's address was accepted")
	}
	// Another link from the same address too.
	if out := r.HandleLink(wire.NewKeepalive(2), addr(2), &fakeLink{name: "other"}); out != nil {
		t.Fatal("another link was accepted")
	}
	expectDropped(t, r, DropNotBound, 2)
	// The link itself, after QUIC moved its connection, still is.
	bind(t, r, 1, addr(1), 1)
	out := r.HandleLink(audioPacket(2, 0, 0, 40), addr(7), browser)
	if len(out) != 1 || out[0].To != addr(1) {
		t.Fatalf("audio from the moved link went to %v", out)
	}
}

func TestUnlinkStopsSendingToAClosedLink(t *testing.T) {
	r, _ := newTestRelay(t)
	browser := &fakeLink{name: "browser"}
	bind(t, r, 1, addr(1), 1)
	bindLink(t, r, 2, browser, 2, 1)

	r.Unlink(browser)
	if got := destinations(r.Handle(audioPacket(1, 0, 0, 40), addr(1))); len(got) != 0 {
		t.Fatalf("audio still went to %v", got)
	}
	if st := r.Stats(); st.Sessions != 4 || st.BoundSessions != 1 {
		t.Fatalf("relay %+v, want the session kept but unbound", st)
	}
	// Its client may bind again, on a new connection, with a new hello.
	bindLink(t, r, 2, &fakeLink{name: "again"}, 2, 2)
}
