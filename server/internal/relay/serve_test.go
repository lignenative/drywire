package relay

import (
	"bytes"
	"io"
	"log/slog"
	"net"
	"slices"
	"testing"
	"time"

	"github.com/lignenative/drywire/server/internal/vectors"
	"github.com/lignenative/drywire/server/internal/wire"
)

// End-to-end tests: a real relay on a UDP socket, and fake clients that
// send the shared test vectors.

const vectorsDir = "../../../core/drywire-proto/vectors"

// fakeClient is a UDP socket that plays the part of a Drywire client.
type fakeClient struct {
	t    *testing.T
	conn *net.UDPConn
}

func newFakeClient(t *testing.T, relayAddr *net.UDPAddr) *fakeClient {
	t.Helper()
	conn, err := net.DialUDP("udp", nil, relayAddr)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { conn.Close() })
	return &fakeClient{t: t, conn: conn}
}

func (c *fakeClient) send(packet []byte) {
	c.t.Helper()
	if _, err := c.conn.Write(packet); err != nil {
		c.t.Fatal(err)
	}
}

// receive waits for one datagram, or returns nil after the timeout.
func (c *fakeClient) receive(timeout time.Duration) []byte {
	buf := make([]byte, 2048)
	c.conn.SetReadDeadline(time.Now().Add(timeout))
	n, err := c.conn.Read(buf)
	if err != nil {
		return nil
	}
	return buf[:n]
}

func (c *fakeClient) expect(want []byte, what string) {
	c.t.Helper()
	got := c.receive(2 * time.Second)
	if !bytes.Equal(got, want) {
		c.t.Fatalf("%s: received %x, want %x", what, got, want)
	}
}

func (c *fakeClient) expectNothing(what string) {
	c.t.Helper()
	if got := c.receive(150 * time.Millisecond); got != nil {
		c.t.Fatalf("%s: received %x, want nothing", what, got)
	}
}

// startRelay serves f on an ephemeral loopback port and returns its address.
// startRelay serves a relay with these sessions, all in one room.
func startRelay(t *testing.T, secrets map[uint32][]byte) *net.UDPAddr {
	t.Helper()
	conn, err := net.ListenUDP("udp", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1)})
	if err != nil {
		t.Fatal(err)
	}
	r := New(time.Now, slog.New(slog.NewTextHandler(io.Discard, nil)))
	for session, secret := range secrets {
		if err := r.AddSession("vectors", session, secret); err != nil {
			t.Fatal(err)
		}
	}
	done := make(chan error, 1)
	go func() { done <- r.Serve(conn) }()
	t.Cleanup(func() {
		conn.Close()
		if err := <-done; err != nil {
			t.Errorf("Serve returned %v", err)
		}
	})
	return conn.LocalAddr().(*net.UDPAddr)
}

// The relay must answer the hello vector with exactly the hello_ack vector,
// forward the audio vector byte for byte, answer the ping vector, and stay
// silent for every invalid vector.
func TestRelayWithSharedVectors(t *testing.T) {
	m, files, err := vectors.Load(vectorsDir)
	if err != nil {
		t.Fatal(err)
	}
	byName := map[string]vectors.Vector{}
	for _, v := range m.Vectors {
		byName[v.Name] = v
	}
	file := func(name string) []byte {
		t.Helper()
		b, ok := files[name+".bin"]
		if !ok {
			t.Fatalf("no vector %q", name)
		}
		return b
	}

	// The vectors' session, with its secret, plus a listener in the room.
	listenerSecret := bytes.Repeat([]byte{0xee}, wire.SecretSize)
	const listener = 7
	relayAddr := startRelay(t, map[uint32][]byte{
		m.Inputs.Session: m.Inputs.SessionSecret,
		listener:         listenerSecret,
	})
	sender := newFakeClient(t, relayAddr)
	other := newFakeClient(t, relayAddr)

	// Before the hello, the relay ignores the sender.
	sender.send(file("ping"))
	sender.expectNothing("ping before hello")

	sender.send(file("hello"))
	sender.expect(file("hello-ack"), "hello")
	other.send(wire.NewHello(listener, 1, listenerSecret))
	other.expect(wire.NewHelloAck(listener, 1, listenerSecret), "listener hello")

	// Replaying the same hello is refused.
	sender.send(file("hello"))
	sender.expectNothing("replayed hello")

	for _, name := range []string{"announce", "audio", "audio-redundant", "audio-epoch-1-end-of-spurt"} {
		sender.send(file(name))
		other.expect(file(name), "forwarded "+name)
		sender.expectNothing("own audio echoed back: " + name)
	}

	// The audio vectors are sequences 7 and 8 of epoch 0, then 0 of epoch 1:
	// nothing is lost.
	sender.send(file("ping"))
	pong := sender.receive(2 * time.Second)
	if pong == nil {
		t.Fatal("no pong")
	}
	got := wire.DecodePong(pong)
	want := wire.Pong{Number: 3, ClientTime: byName["ping"].Ping.ClientTime, AudioReceived: 3, AudioLost: 0}
	if got != want {
		t.Fatalf("pong = %+v, want %+v", got, want)
	}

	// Invalid packets never get an answer and are never forwarded. The
	// vectors that only receivers can reject (bad tag, bad framing, bad
	// signature) are forwarded like any other: the relay cannot tell.
	receiverOnly := []string{string(wire.ErrBadTag), string(wire.ErrBadFraming), string(wire.ErrBadSignature)}
	for _, v := range m.Vectors {
		if v.Error == "" || slices.Contains(receiverOnly, v.Error) {
			continue
		}
		sender.send(files[v.File])
		sender.expectNothing("answer to " + v.Name)
		other.expectNothing("forwarded " + v.Name)
	}
}
