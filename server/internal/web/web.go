// Package web lets browsers reach the relay. Browsers cannot send UDP, so
// they open a WebTransport session (HTTP/3 over QUIC) and send the very
// same Drywire packets as its datagrams (ADR 0038). The relay binds a
// browser's session to that WebTransport session, and copies packets
// between browsers and native clients alike.
//
// The server presents a certificate it makes itself, valid for less than
// two weeks, which browsers accept when the page names its hash; the
// control plane hands out those hashes (ADR 0039).
package web

import (
	"context"
	"crypto/tls"
	"errors"
	"log/slog"
	"net"
	"net/http"
	"net/netip"
	"time"

	"github.com/quic-go/quic-go"
	"github.com/quic-go/quic-go/http3"
	"github.com/quic-go/webtransport-go"

	"github.com/lignenative/drywire/server/internal/relay"
)

// Path is where browsers open their WebTransport session.
const Path = "/v1/relay"

// Server accepts WebTransport sessions and feeds their datagrams to the
// relay.
type Server struct {
	relay *relay.Relay
	udp   *net.UDPConn
	log   *slog.Logger
	certs *certificates
	wt    *webtransport.Server
}

// New prepares a server for the relay r, which sends to native clients on
// udp. hosts are the names and addresses browsers use to reach it, which
// its certificates carry.
func New(r *relay.Relay, udp *net.UDPConn, hosts []string, now func() time.Time, log *slog.Logger) (*Server, error) {
	certs, err := newCertificates(now, hosts)
	if err != nil {
		return nil, err
	}
	s := &Server{relay: r, udp: udp, log: log, certs: certs}
	mux := http.NewServeMux()
	mux.HandleFunc(Path, s.session)
	h3 := &http3.Server{
		Handler: mux,
		Logger:  log,
		TLSConfig: http3.ConfigureTLSConfig(&tls.Config{
			MinVersion:     tls.VersionTLS13,
			GetCertificate: certs.get,
		}),
		QUICConfig: &quic.Config{
			EnableDatagrams: true,
			// A browser sends a keepalive every 10 seconds while its gate
			// is closed (protocol section 6.3); the connection outlives it.
			MaxIdleTimeout: 30 * time.Second,
		},
	}
	webtransport.ConfigureHTTP3Server(h3)
	s.wt = &webtransport.Server{
		H3: h3,
		// Any page may open a session: it gets nothing until a hello
		// proves it holds a session secret from the control plane, and no
		// cookie or other ambient credential rides on the connection.
		CheckOrigin: func(*http.Request) bool { return true },
	}
	return s, nil
}

// CertificateHashes are the SHA-256 hashes, in hex, a browser passes as
// serverCertificateHashes to reach this server.
func (s *Server) CertificateHashes() []string { return s.certs.Hashes() }

// Serve answers on conn until Close.
func (s *Server) Serve(conn net.PacketConn) error {
	err := s.wt.Serve(conn)
	// Close cancels the server's context, which is how Serve ends.
	if errors.Is(err, http.ErrServerClosed) || errors.Is(err, quic.ErrServerClosed) || errors.Is(err, context.Canceled) {
		return nil
	}
	return err
}

// Close stops the server and ends every session.
func (s *Server) Close() error { return s.wt.Close() }

// session serves one browser: every datagram goes to the relay, like a
// UDP packet, and the relay's answers go back through Deliver.
func (s *Server) session(w http.ResponseWriter, r *http.Request) {
	session, err := s.wt.Upgrade(w, r)
	if err != nil {
		w.WriteHeader(http.StatusBadRequest)
		return
	}
	// The session itself is the link: a pointer, so the relay tells two
	// browsers apart even behind one address.
	defer s.relay.Unlink(session)
	ctx := session.Context()
	for {
		packet, err := session.ReceiveDatagram(ctx)
		if err != nil {
			return // the browser closed the session, or the server stops
		}
		from := remote(session.RemoteAddr())
		s.relay.Deliver(s.udp, s.relay.HandleLink(packet, from, session))
	}
}

// remote is the address a QUIC connection comes from, for the relay's
// per-address rate limit.
func remote(a net.Addr) netip.AddrPort {
	if u, ok := a.(*net.UDPAddr); ok {
		return u.AddrPort()
	}
	return netip.AddrPort{}
}
