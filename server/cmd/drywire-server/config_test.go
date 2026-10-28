package main

import (
	"net"
	"net/netip"
	"strings"
	"testing"
	"time"
)

func TestTrustedProxiesArePrivateNetworksOnly(t *testing.T) {
	for v, want := range map[string]string{
		"":                       "",
		"1":                      "127.0.0.0/8 ::1/128",
		"172.18.0.0/16":          "127.0.0.0/8 ::1/128 172.18.0.0/16",
		"172.18.0.5, fd00::/64":  "127.0.0.0/8 ::1/128 172.18.0.5/32 fd00::/64",
		"10.1.2.3/8":             "127.0.0.0/8 ::1/128 10.0.0.0/8",
		"203.0.113.0/24":         "error",
		"172.0.0.0/8":            "error", // wider than 172.16.0.0/12
		"0.0.0.0/0":              "error",
		"yes":                    "error",
		"192.168.1.0/24,8.8.8.8": "error",
	} {
		proxies, err := trustedProxies(v)
		got := ""
		if err != nil {
			got = "error"
		} else {
			var s []string
			for _, p := range proxies {
				s = append(s, p.String())
			}
			got = strings.Join(s, " ")
		}
		if got != want {
			t.Errorf("DRYWIRE_TRUST_PROXY=%q: %s, want %s", v, got, want)
		}
	}
}

func TestPlainHTTPLeavesLoopbackOnlyForANamedProxyNetwork(t *testing.T) {
	t.Setenv("DRYWIRE_HTTP_ADDR", "0.0.0.0:10050")
	t.Setenv("DRYWIRE_ACCESS_LOG", "off")
	for v, ok := range map[string]bool{"": false, "1": false, "172.18.0.0/16": true} {
		t.Setenv("DRYWIRE_TRUST_PROXY", v)
		c, err := loadConfig()
		if (err == nil) != ok || c.proxyOnly != ok {
			t.Errorf("DRYWIRE_TRUST_PROXY=%q: error %v, proxyOnly %v", v, err, c.proxyOnly)
		}
	}
	t.Setenv("DRYWIRE_HTTP_ADDR", "127.0.0.1:10050")
	if c, err := loadConfig(); err != nil || c.proxyOnly {
		t.Errorf("on loopback: error %v, proxyOnly %v", err, c.proxyOnly)
	}
}

// serveOnce runs a proxyOnly listener on loopback with the allowed
// networks, connects to it once, and says whether the connection was
// accepted, or closed at once.
func serveOnce(t *testing.T, allowed ...string) (accepted bool) {
	t.Helper()
	inner, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	l := proxyOnly{Listener: inner}
	for _, a := range allowed {
		l.allowed = append(l.allowed, netip.MustParsePrefix(a))
	}
	defer l.Close()
	conns := make(chan net.Conn, 1)
	go func() {
		if c, err := l.Accept(); err == nil {
			conns <- c
		}
	}()
	c, err := net.Dial("tcp", inner.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	select {
	case got := <-conns:
		got.Close()
		return true
	case <-time.After(500 * time.Millisecond):
	}
	// Not accepted: the server side must have closed it.
	c.SetReadDeadline(time.Now().Add(2 * time.Second))
	if _, err := c.Read(make([]byte, 1)); err == nil || strings.Contains(err.Error(), "timeout") {
		t.Fatalf("a refused connection was left open: %v", err)
	}
	return false
}

func TestProxyOnlyAcceptsTheProxysNetworksOnly(t *testing.T) {
	if serveOnce(t, "172.18.0.0/16") {
		t.Error("a connection from outside the proxy's network was accepted")
	}
	if !serveOnce(t, "172.18.0.0/16", "127.0.0.0/8") {
		t.Error("a connection from an allowed network was refused")
	}
}
