package web

import (
	"bytes"
	"context"
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"encoding/hex"
	"errors"
	"io"
	"log/slog"
	"net"
	"slices"
	"testing"
	"time"

	"github.com/quic-go/quic-go"
	"github.com/quic-go/quic-go/http3"
	"github.com/quic-go/webtransport-go"

	"github.com/lignenative/drywire/server/internal/relay"
	"github.com/lignenative/drywire/server/internal/wire"
)

// A browser, played by webtransport-go's client, and a native client on
// UDP share a room through the real relay and the real WebTransport
// server: each hears the other's packets byte for byte.

func secret(b byte) []byte { return bytes.Repeat([]byte{b}, wire.SecretSize) }

func audio(session, seq uint32) []byte {
	h := wire.Header{Version: wire.Version, Type: wire.TypeAudio, Session: session,
		Sequence: seq, Timestamp: seq * 240}.Bytes()
	return append(h, bytes.Repeat([]byte{0xab}, 40)...)
}

// start serves a relay with sessions 1 (the browser) and 2 (UDP) in one
// room, on loopback ports the system picks.
func start(t *testing.T) (*Server, *net.UDPAddr, *net.UDPAddr) {
	t.Helper()
	log := slog.New(slog.NewTextHandler(io.Discard, nil))
	r := relay.New(time.Now, log)
	for id, b := range map[uint32]byte{1: 0x11, 2: 0x22} {
		if err := r.AddSession("room", id, secret(b)); err != nil {
			t.Fatal(err)
		}
	}
	udp, err := net.ListenUDP("udp", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1)})
	if err != nil {
		t.Fatal(err)
	}
	quicConn, err := net.ListenUDP("udp", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1)})
	if err != nil {
		t.Fatal(err)
	}
	s, err := New(r, udp, []string{"127.0.0.1"}, time.Now, log)
	if err != nil {
		t.Fatal(err)
	}
	done := make(chan error, 2)
	go func() { done <- r.Serve(udp) }()
	go func() { done <- s.Serve(quicConn) }()
	t.Cleanup(func() {
		s.Close()
		udp.Close()
		quicConn.Close()
		for range 2 {
			if err := <-done; err != nil {
				t.Errorf("serve: %v", err)
			}
		}
	})
	return s, udp.LocalAddr().(*net.UDPAddr), quicConn.LocalAddr().(*net.UDPAddr)
}

// dial opens a WebTransport session as a browser does with
// serverCertificateHashes: the certificate is accepted only if its hash
// is one the server advertises.
func dial(t *testing.T, s *Server, addr *net.UDPAddr) *webtransport.Session {
	t.Helper()
	hashes := s.CertificateHashes()
	d := &webtransport.Transport{
		TLSClientConfig: &tls.Config{
			NextProtos:         []string{http3.NextProtoH3},
			InsecureSkipVerify: true, // replaced by the hash check below
			VerifyPeerCertificate: func(raw [][]byte, _ [][]*x509.Certificate) error {
				sum := sha256.Sum256(raw[0])
				if !slices.Contains(hashes, hex.EncodeToString(sum[:])) {
					return errors.New("certificate hash not advertised")
				}
				return nil
			},
		},
		QUICConfig: &quic.Config{EnableDatagrams: true, EnableStreamResetPartialDelivery: true},
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_, session, err := d.Dial(ctx, "https://"+addr.String()+Path, nil)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { session.CloseWithError(0, "") })
	return session
}

func receive(t *testing.T, s *webtransport.Session) []byte {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	b, err := s.ReceiveDatagram(ctx)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

func TestBrowserAndNativeClientShareARoom(t *testing.T) {
	s, relayAddr, webAddr := start(t)
	browser := dial(t, s, webAddr)
	native, err := net.DialUDP("udp", nil, relayAddr)
	if err != nil {
		t.Fatal(err)
	}
	defer native.Close()
	nativeReceive := func() []byte {
		buf := make([]byte, 1500)
		native.SetReadDeadline(time.Now().Add(2 * time.Second))
		n, err := native.Read(buf)
		if err != nil {
			t.Fatal(err)
		}
		return buf[:n]
	}

	// Both bind their sessions with the same hello.
	if err := browser.SendDatagram(wire.NewHello(1, 1, secret(0x11))); err != nil {
		t.Fatal(err)
	}
	if got := receive(t, browser); !bytes.Equal(got, wire.NewHelloAck(1, 1, secret(0x11))) {
		t.Fatalf("browser hello answered with %x", got)
	}
	native.Write(wire.NewHello(2, 1, secret(0x22)))
	if got := nativeReceive(); !bytes.Equal(got, wire.NewHelloAck(2, 1, secret(0x22))) {
		t.Fatalf("native hello answered with %x", got)
	}

	// Audio crosses both ways, unchanged.
	browser.SendDatagram(audio(1, 0))
	if got := nativeReceive(); !bytes.Equal(got, audio(1, 0)) {
		t.Fatalf("native received %x", got)
	}
	native.Write(audio(2, 0))
	if got := receive(t, browser); !bytes.Equal(got, audio(2, 0)) {
		t.Fatalf("browser received %x", got)
	}
}

func TestADatagramBeforeHelloGetsNothing(t *testing.T) {
	s, _, webAddr := start(t)
	browser := dial(t, s, webAddr)
	browser.SendDatagram(wire.NewPing(1, 1, 0))
	ctx, cancel := context.WithTimeout(context.Background(), 200*time.Millisecond)
	defer cancel()
	if b, err := browser.ReceiveDatagram(ctx); err == nil {
		t.Fatalf("received %x before any hello", b)
	}
}

func TestCertificatesRotateAndStayAdvertised(t *testing.T) {
	now := time.Unix(1_800_000_000, 0)
	c, err := newCertificates(func() time.Time { return now }, []string{"127.0.0.1", "voice.example.org"})
	if err != nil {
		t.Fatal(err)
	}
	first := c.Hashes()
	presented := func() string {
		cert, err := c.get(nil)
		if err != nil {
			t.Fatal(err)
		}
		x, _ := x509.ParseCertificate(cert.Certificate[0])
		if x.NotAfter.Sub(x.NotBefore) >= 14*24*time.Hour || now.Before(x.NotBefore) || now.After(x.NotAfter) {
			t.Fatalf("validity %v to %v at %v: browsers refuse it", x.NotBefore, x.NotAfter, now)
		}
		sum := sha256.Sum256(cert.Certificate[0])
		return hex.EncodeToString(sum[:])
	}
	if presented() != first[0] {
		t.Fatal("the presented certificate is not the first hash")
	}
	now = now.Add(rotateAfter)
	// A client that learned the hashes before the rotation still connects.
	if p := presented(); p != first[1] {
		t.Fatalf("after rotation presented %s, want the advertised next %s", p, first[1])
	}
	if h := c.Hashes(); h[0] != first[1] || h[1] == first[0] {
		t.Fatalf("hashes after rotation %v", h)
	}
}
