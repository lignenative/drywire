package main

import (
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net"
	"net/netip"
	"net/url"
	"os"
	"strconv"
	"strings"

	"github.com/lignenative/drywire/server/internal/api"
	"github.com/lignenative/drywire/server/internal/auth"
	"github.com/lignenative/drywire/server/internal/store"
)

const defaultAddr = "127.0.0.1:10050"

// config is the server's configuration, read from the environment:
//
//	DRYWIRE_UDP_ADDR        relay address, default 127.0.0.1:10050
//	DRYWIRE_HTTP_ADDR       control plane address, default 127.0.0.1:10050
//	DRYWIRE_PUBLIC_URL      the instance's origin as clients reach it,
//	                        default http://<DRYWIRE_HTTP_ADDR>
//	DRYWIRE_PUBLIC_RELAY    the relay's address as clients reach it,
//	                        default DRYWIRE_UDP_ADDR
//	DRYWIRE_DB              SQLite database, default drywire.db
//	DRYWIRE_LEDGER          deletion ledger, default <DRYWIRE_DB>.deletions
//	DRYWIRE_SPACE_CREATORS  who may create spaces: * for anyone, or public
//	                        keys in hex separated by commas; default nobody
//	DRYWIRE_MAX_MEMBERS     member limit of a new space, default 50
//	DRYWIRE_TLS_CERT and DRYWIRE_TLS_KEY
//	                        serve HTTPS (TLS 1.3) from these PEM files
//	DRYWIRE_TRUST_PROXY     1 behind a reverse proxy on this host: the
//	                        client address comes from X-Forwarded-For
//	DRYWIRE_ACCESS_LOG      access log file, - for standard output (the
//	                        default), off for none
//	DRYWIRE_ROOMS           phase 1 rooms file (optional)
type config struct {
	udpAddr, httpAddr  string
	publicURL          string
	dbPath, ledgerPath string
	tlsCert, tlsKey    string
	roomsPath          string
	accessLog          *slog.Logger
	api                api.Config
	warnings           []string
}

func envOr(name, fallback string) string {
	if v := os.Getenv(name); v != "" {
		return v
	}
	return fallback
}

func loadConfig() (config, error) {
	c := config{
		udpAddr:   envOr("DRYWIRE_UDP_ADDR", defaultAddr),
		httpAddr:  envOr("DRYWIRE_HTTP_ADDR", defaultAddr),
		dbPath:    envOr("DRYWIRE_DB", "drywire.db"),
		tlsCert:   os.Getenv("DRYWIRE_TLS_CERT"),
		tlsKey:    os.Getenv("DRYWIRE_TLS_KEY"),
		roomsPath: os.Getenv("DRYWIRE_ROOMS"),
	}
	c.ledgerPath = envOr("DRYWIRE_LEDGER", c.dbPath+".deletions")
	if (c.tlsCert == "") != (c.tlsKey == "") {
		return c, errors.New("set both DRYWIRE_TLS_CERT and DRYWIRE_TLS_KEY, or neither")
	}

	// Bearer tokens and session secrets cross this connection: in plain
	// HTTP it must not leave the machine (a reverse proxy on this host
	// terminates TLS in front of it).
	host, _, err := net.SplitHostPort(c.httpAddr)
	if err != nil {
		return c, fmt.Errorf("DRYWIRE_HTTP_ADDR: %w", err)
	}
	if ip, err := netip.ParseAddr(host); c.tlsCert == "" && (err != nil || !ip.IsLoopback()) {
		return c, fmt.Errorf("DRYWIRE_HTTP_ADDR %s is not a loopback address: serve TLS (DRYWIRE_TLS_CERT and DRYWIRE_TLS_KEY) or put a reverse proxy on this host in front of 127.0.0.1", c.httpAddr)
	}

	public := os.Getenv("DRYWIRE_PUBLIC_URL")
	if public == "" {
		public = "http://" + c.httpAddr
	}
	if c.publicURL, err = auth.CanonicalOrigin(public); err != nil {
		return c, fmt.Errorf("DRYWIRE_PUBLIC_URL: %w", err)
	}
	if u, _ := url.Parse(c.publicURL); u.Scheme != "https" {
		if ip, err := netip.ParseAddr(u.Hostname()); (err != nil || !ip.IsLoopback()) && u.Hostname() != "localhost" {
			return c, fmt.Errorf("DRYWIRE_PUBLIC_URL %s: clients only use plain http with this machine; use https://", c.publicURL)
		}
	}

	c.api = api.Config{
		ServerVersion: version,
		RelayAddr:     envOr("DRYWIRE_PUBLIC_RELAY", c.udpAddr),
		MaxMembers:    50,
		TrustProxy:    os.Getenv("DRYWIRE_TRUST_PROXY") == "1",
	}
	if v := os.Getenv("DRYWIRE_MAX_MEMBERS"); v != "" {
		n, err := strconv.Atoi(v)
		if err != nil || n < 2 || n > 10_000 {
			return c, errors.New("DRYWIRE_MAX_MEMBERS must be a number from 2 to 10000")
		}
		c.api.MaxMembers = n
	}
	switch creators := strings.TrimSpace(os.Getenv("DRYWIRE_SPACE_CREATORS")); creators {
	case "":
		c.warnings = append(c.warnings, "DRYWIRE_SPACE_CREATORS is empty: nobody can create a space")
	case "*":
		c.api.AnyoneCreates = true
		c.warnings = append(c.warnings, "DRYWIRE_SPACE_CREATORS is *: anyone who reaches this server can create spaces")
	default:
		for _, k := range strings.Split(creators, ",") {
			key, err := store.ParseKey(strings.TrimSpace(k))
			if err != nil {
				return c, fmt.Errorf("DRYWIRE_SPACE_CREATORS: %w", err)
			}
			c.api.SpaceCreators = append(c.api.SpaceCreators, key)
		}
	}

	switch path := envOr("DRYWIRE_ACCESS_LOG", "-"); path {
	case "off":
	case "-":
		c.accessLog = slog.New(slog.NewJSONHandler(os.Stdout, nil))
	default:
		f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
		if err != nil {
			return c, fmt.Errorf("DRYWIRE_ACCESS_LOG: %w", err)
		}
		c.accessLog = slog.New(slog.NewJSONHandler(io.Writer(f), nil))
	}
	return c, nil
}
