package main

import (
	"net"
	"net/netip"
)

// proxyOnly accepts connections from the given networks only, and closes
// any other at once. The server wraps its HTTP listener in it when it
// serves plain HTTP on a private network (DRYWIRE_TRUST_PROXY): bearer
// tokens then cross that network unencrypted, so the control plane talks
// to its reverse proxy, and to no other machine there.
type proxyOnly struct {
	net.Listener
	allowed []netip.Prefix
}

func (l proxyOnly) Accept() (net.Conn, error) {
	for {
		conn, err := l.Listener.Accept()
		if err != nil {
			return nil, err
		}
		if peer, err := netip.ParseAddrPort(conn.RemoteAddr().String()); err == nil {
			for _, p := range l.allowed {
				if p.Contains(peer.Addr().Unmap()) {
					return conn, nil
				}
			}
		}
		conn.Close()
	}
}
