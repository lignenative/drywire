package api

import (
	"bufio"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"net/netip"
	"strings"
	"testing"
	"time"

	"github.com/lignenative/drywire/server/internal/presence"
	"github.com/lignenative/drywire/server/internal/relay"
	"github.com/lignenative/drywire/server/internal/store"
	"github.com/lignenative/drywire/server/internal/wire"
)

// stream is an open event stream of one space.
type stream struct {
	t      *testing.T
	resp   *http.Response
	events chan sse
}

type sse struct {
	name string
	data string
}

func (c *client) subscribe(space store.ID) *stream {
	c.h.t.Helper()
	req, _ := http.NewRequest("GET", c.h.url+"/v1/spaces/"+space.String()+"/events", nil)
	req.Header.Set("Authorization", "Bearer "+c.token)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		c.h.t.Fatal(err)
	}
	if resp.StatusCode != http.StatusOK || resp.Header.Get("Content-Type") != "text/event-stream" {
		c.h.t.Fatalf("events: status %d, %s", resp.StatusCode, resp.Header.Get("Content-Type"))
	}
	s := &stream{t: c.h.t, resp: resp, events: make(chan sse, 64)}
	go func() {
		defer close(s.events)
		lines := bufio.NewScanner(resp.Body)
		var e sse
		for lines.Scan() {
			line := lines.Text()
			switch {
			case strings.HasPrefix(line, "event: "):
				e.name = strings.TrimPrefix(line, "event: ")
			case strings.HasPrefix(line, "data: "):
				e.data = strings.TrimPrefix(line, "data: ")
			case line == "" && e.name != "":
				s.events <- e
				e = sse{}
			}
		}
	}()
	c.h.t.Cleanup(func() { resp.Body.Close() })
	return s
}

// next returns the next event. The timeout only turns a missing event
// into a failure instead of a hung test; nothing depends on timing.
func (s *stream) next() (sse, bool) {
	s.t.Helper()
	select {
	case e, ok := <-s.events:
		return e, ok
	case <-time.After(5 * time.Second):
		s.t.Fatal("no event")
		return sse{}, false
	}
}

func (s *stream) expect(name string, into any) {
	s.t.Helper()
	e, ok := s.next()
	if !ok {
		s.t.Fatalf("the stream ended, want %s", name)
	}
	if e.name != name {
		s.t.Fatalf("event %s %s, want %s", e.name, e.data, name)
	}
	if into != nil {
		if err := json.Unmarshal([]byte(e.data), into); err != nil {
			s.t.Fatal(err)
		}
	}
}

// expectEnd checks that the server closed the stream.
func (s *stream) expectEnd() {
	s.t.Helper()
	if e, ok := s.next(); ok {
		s.t.Fatalf("event %s %s, want the end of the stream", e.name, e.data)
	}
}

func (c *client) join(room store.ID) JoinAnswer {
	c.h.t.Helper()
	var a JoinAnswer
	c.must(200, "POST", "/v1/rooms/"+room.String()+"/join", nil, &a)
	return a
}

// team makes a space owned by alex with sam as a member.
func team(t *testing.T, h *harness) (alex, sam *client, v SpaceView) {
	t.Helper()
	alex, sam = h.client(1).login(), h.client(2).login()
	v = alex.createSpace("Team")
	if status := sam.redeem(alex.invite(v.Space.ID, 1), "sam"); status != http.StatusOK {
		t.Fatal(status)
	}
	return alex, sam, v
}

func TestJoiningARoomGivesAWorkingRelaySession(t *testing.T) {
	h := newHarness(t, Config{AnyoneCreates: true})
	alex, sam, v := team(t, h)
	room := v.Rooms[0]

	a := alex.join(room.ID)
	if a.Session == 0 || a.Relay != "voice.example.org:10050" || a.Room != room || len(a.Sessions) != 1 {
		t.Fatalf("join answer %+v", a)
	}
	b := sam.join(room.ID)
	if len(b.Sessions) != 2 {
		t.Fatalf("sam sees %+v", b.Sessions)
	}

	// The secret opens the session on the relay, and the two are in the
	// same relay room.
	secret, _ := hex.DecodeString(b.SessionSecret)
	from := netip.MustParseAddrPort("10.0.0.2:5000")
	out := h.relay.Handle(wire.NewHello(b.Session, 1, secret), from)
	if len(out) != 1 || out[0].To != from {
		t.Fatalf("hello answered with %v", out)
	}
	if st := h.relay.Stats(); st.Rooms != 1 || st.Sessions != 2 {
		t.Fatalf("relay %+v", st)
	}

	// An outsider cannot join.
	kim := h.client(3).login()
	if status := kim.do("POST", "/v1/rooms/"+room.ID.String()+"/join", nil, nil); status != http.StatusNotFound {
		t.Fatalf("outsider joins: %d", status)
	}
}

func TestPresenceFollowsJoinsLeavesAndDisconnects(t *testing.T) {
	h := newHarness(t, Config{AnyoneCreates: true})
	alex, sam, v := team(t, h)
	room := v.Rooms[0]
	var scrims store.Room
	alex.must(201, "POST", "/v1/spaces/"+v.Space.ID.String()+"/rooms", map[string]string{"name": "scrims"}, &scrims)

	a := alex.join(room.ID)
	events := sam.subscribe(v.Space.ID)
	var snap struct {
		Sessions []presence.Session `json:"sessions"`
	}
	events.expect("snapshot", &snap)
	if len(snap.Sessions) != 1 || snap.Sessions[0].Session != a.Session || snap.Sessions[0].Name != "owner" {
		t.Fatalf("snapshot %+v", snap)
	}

	var e presence.Event
	b := sam.join(room.ID)
	events.expect(presence.Join, &e)
	if e.Session != b.Session || e.Name != "sam" || e.PublicKey != sam.key || e.Room != room.ID {
		t.Fatalf("join %+v", e)
	}

	// Moving to another room is a leave, then a join: one session each.
	b2 := sam.join(scrims.ID)
	events.expect(presence.Leave, &e)
	if e.Session != b.Session {
		t.Fatalf("leave %+v", e)
	}
	events.expect(presence.Join, &e)
	if e.Session != b2.Session || e.Room != scrims.ID {
		t.Fatalf("join %+v", e)
	}
	if st := h.relay.Stats(); st.Sessions != 2 {
		t.Fatalf("the old session is still on the relay: %+v", st)
	}

	sam.must(204, "POST", "/v1/leave", nil, nil)
	events.expect(presence.Leave, &e)

	// Alex never said hello: the relay drops the session after its
	// timeout, and the room hears of a disconnect.
	h.clock.advance(relay.SessionTimeout + time.Second)
	h.server.ExpireSessions()
	events.expect(presence.Disconnect, &e)
	if e.Session != a.Session {
		t.Fatalf("disconnect %+v", e)
	}
}

func TestRemovalEndsTheSessionAndTheStream(t *testing.T) {
	h := newHarness(t, Config{AnyoneCreates: true})
	alex, sam, v := team(t, h)
	b := sam.join(v.Rooms[0].ID)
	samEvents := sam.subscribe(v.Space.ID)
	samEvents.expect("snapshot", nil)
	alexEvents := alex.subscribe(v.Space.ID)
	alexEvents.expect("snapshot", nil)

	alex.must(204, "DELETE", "/v1/spaces/"+v.Space.ID.String()+"/members/"+sam.key.String(), nil, nil)
	var e presence.Event
	alexEvents.expect(presence.Leave, &e)
	if e.Session != b.Session {
		t.Fatalf("leave %+v", e)
	}
	samEvents.expect(presence.Leave, nil)
	samEvents.expectEnd()
	if st := h.relay.Stats(); st.Sessions != 0 {
		t.Fatalf("the removed member's session is still on the relay: %+v", st)
	}

	// Deleting the space closes every stream.
	alex.must(204, "DELETE", "/v1/spaces/"+v.Space.ID.String(), nil, nil)
	alexEvents.expectEnd()
}

func TestDeletingARoomEmptiesIt(t *testing.T) {
	h := newHarness(t, Config{AnyoneCreates: true})
	alex, sam, v := team(t, h)
	var scrims store.Room
	alex.must(201, "POST", "/v1/spaces/"+v.Space.ID.String()+"/rooms", map[string]string{"name": "scrims"}, &scrims)
	sam.join(scrims.ID)
	alex.must(204, "DELETE", "/v1/rooms/"+scrims.ID.String(), nil, nil)
	if st := h.relay.Stats(); st.Sessions != 0 {
		t.Fatalf("sessions left in a deleted room: %+v", st)
	}
	if status := sam.do("POST", "/v1/rooms/"+scrims.ID.String()+"/join", nil, nil); status != http.StatusNotFound {
		t.Fatalf("join a deleted room: %d", status)
	}
}
