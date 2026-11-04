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
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/lignenative/drywire/server/internal/accesslog"
	"github.com/lignenative/drywire/server/internal/api"
	"github.com/lignenative/drywire/server/internal/auth"
	"github.com/lignenative/drywire/server/internal/mail"
	"github.com/lignenative/drywire/server/internal/store"
	"github.com/lignenative/drywire/server/internal/web"
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
//	DRYWIRE_WEB_ADDR        UDP address of the relay's WebTransport side,
//	                        for browsers (ADR 0038); default none: no
//	                        browser can join (make dev: 127.0.0.1:10058)
//	DRYWIRE_PUBLIC_WEB_RELAY
//	                        that address as browsers reach it, host:port,
//	                        default DRYWIRE_WEB_ADDR
//	DRYWIRE_DB              SQLite database, default drywire.db
//	DRYWIRE_LEDGER          deletion ledger, default <DRYWIRE_DB>.deletions
//	DRYWIRE_SPACE_CREATORS  who may create spaces: * for anyone, or public
//	                        keys in hex separated by commas; default every
//	                        account with a user name (ADR 0055)
//	DRYWIRE_MAX_MEMBERS     member limit of a new space, default 50
//	DRYWIRE_SMTP_ADDR       where sign-in codes are sent from (ADR 0055): a
//	                        provider's submission port, host:465 or :587,
//	                        with DRYWIRE_SMTP_USER, DRYWIRE_SMTP_PASSWORD and
//	                        DRYWIRE_MAIL_FROM ("Drywire <no-reply@…>")
//	DRYWIRE_MAIL_DIR        or, in development, a directory the codes are
//	                        written to; with neither, no e-mail sign-in
//	DRYWIRE_TLS_CERT and DRYWIRE_TLS_KEY
//	                        serve HTTPS (TLS 1.3) from these PEM files
//	DRYWIRE_TRUST_PROXY     where the reverse proxy in front connects
//	                        from: 1 for this host's loopback, or private
//	                        networks such as 172.18.0.0/16 (comma-separated)
//	                        for a proxy in a Docker network. Its requests
//	                        name their client in X-Forwarded-For
//	DRYWIRE_ACCESS_LOG      access log: - for standard output (the
//	                        default), off for none, a directory (ending
//	                        in /) for one file per day deleted after
//	                        DRYWIRE_ACCESS_LOG_DAYS (default and most 365),
//	                        or a single file rotated by someone else
type config struct {
	udpAddr, httpAddr  string
	webAddr            string // empty: no WebTransport
	publicURL          string // set by setPublicAddresses
	dbPath, ledgerPath string
	tlsCert, tlsKey    string
	// proxyOnly: plain HTTP on a private network, so the control plane
	// accepts connections from the trusted proxies only (proxyOnly in
	// listener.go).
	proxyOnly bool
	accessLog *slog.Logger
	api       api.Config
	warnings  []string
}

func envOr(name, fallback string) string {
	if v := os.Getenv(name); v != "" {
		return v
	}
	return fallback
}

func loadConfig() (config, error) {
	c := config{
		udpAddr:  envOr("DRYWIRE_UDP_ADDR", defaultAddr),
		httpAddr: envOr("DRYWIRE_HTTP_ADDR", defaultAddr),
		webAddr:  os.Getenv("DRYWIRE_WEB_ADDR"),
		dbPath:   envOr("DRYWIRE_DB", "drywire.db"),
		tlsCert:  os.Getenv("DRYWIRE_TLS_CERT"),
		tlsKey:   os.Getenv("DRYWIRE_TLS_KEY"),
	}
	c.ledgerPath = envOr("DRYWIRE_LEDGER", c.dbPath+".deletions")
	if (c.tlsCert == "") != (c.tlsKey == "") {
		return c, errors.New("set both DRYWIRE_TLS_CERT and DRYWIRE_TLS_KEY, or neither")
	}

	c.api = api.Config{
		ServerVersion: version,
		RelayAddr:     os.Getenv("DRYWIRE_PUBLIC_RELAY"),
		MaxMembers:    50,
	}
	mailer, err := mailSender()
	if err != nil {
		return c, err
	}
	c.api.Mail = mailer
	proxies, err := trustedProxies(os.Getenv("DRYWIRE_TRUST_PROXY"))
	if err != nil {
		return c, fmt.Errorf("DRYWIRE_TRUST_PROXY: %w", err)
	}
	c.api.TrustedProxies = proxies

	// Bearer tokens and session secrets cross this connection: in plain
	// HTTP it must stay between the server and the reverse proxy that
	// terminates TLS in front of it. That is this machine's loopback, or a
	// private network the operator named in DRYWIRE_TRUST_PROXY (a proxy
	// in a Docker network), from which alone connections are accepted.
	host, _, err := net.SplitHostPort(c.httpAddr)
	if err != nil {
		return c, fmt.Errorf("DRYWIRE_HTTP_ADDR: %w", err)
	}
	if ip, err := netip.ParseAddr(host); c.tlsCert == "" && (err != nil || !ip.IsLoopback()) {
		if !slices.ContainsFunc(proxies, func(p netip.Prefix) bool { return !p.Addr().IsLoopback() }) {
			return c, fmt.Errorf("DRYWIRE_HTTP_ADDR %s is not a loopback address: serve TLS (DRYWIRE_TLS_CERT and DRYWIRE_TLS_KEY), put a reverse proxy on this host in front of 127.0.0.1, or name the private network of the proxy in DRYWIRE_TRUST_PROXY", c.httpAddr)
		}
		c.proxyOnly = true
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

	switch path := envOr("DRYWIRE_ACCESS_LOG", "-"); {
	case path == "off":
	case path == "-":
		c.accessLog = slog.New(slog.NewJSONHandler(os.Stdout, nil))
	case strings.HasSuffix(path, "/"):
		days := accesslog.MaxDays
		if v := os.Getenv("DRYWIRE_ACCESS_LOG_DAYS"); v != "" {
			n, err := strconv.Atoi(v)
			if err != nil {
				return c, fmt.Errorf("DRYWIRE_ACCESS_LOG_DAYS: %w", err)
			}
			days = n
		}
		dir, err := accesslog.Open(path, days, time.Now)
		if err != nil {
			return c, fmt.Errorf("DRYWIRE_ACCESS_LOG: %w", err)
		}
		c.accessLog = slog.New(slog.NewJSONHandler(dir, nil))
	default:
		f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
		if err != nil {
			return c, fmt.Errorf("DRYWIRE_ACCESS_LOG: %w", err)
		}
		c.accessLog = slog.New(slog.NewJSONHandler(io.Writer(f), nil))
	}
	return c, nil
}

// privateNetworks are the address blocks a trusted proxy may sit in:
// loopback, and the private ranges of RFC 1918 and RFC 4193, where Docker
// and other local networks live.
var privateNetworks = []netip.Prefix{
	netip.MustParsePrefix("127.0.0.0/8"),
	netip.MustParsePrefix("::1/128"),
	netip.MustParsePrefix("10.0.0.0/8"),
	netip.MustParsePrefix("172.16.0.0/12"),
	netip.MustParsePrefix("192.168.0.0/16"),
	netip.MustParsePrefix("fc00::/7"),
}

// trustedProxies reads DRYWIRE_TRUST_PROXY: empty for no proxy, 1 for a
// proxy on this host's loopback, or comma-separated private networks or
// addresses for a proxy elsewhere on a private network; loopback is then
// trusted too, for the health check. A public network is refused: its
// X-Forwarded-For could come from anyone there.
func trustedProxies(v string) ([]netip.Prefix, error) {
	switch v = strings.TrimSpace(v); v {
	case "", "0":
		return nil, nil
	case "1":
		return privateNetworks[:2], nil
	}
	proxies := slices.Clone(privateNetworks[:2])
	for _, s := range strings.Split(v, ",") {
		s = strings.TrimSpace(s)
		p, err := netip.ParsePrefix(s)
		if err != nil {
			a, aerr := netip.ParseAddr(s)
			if aerr != nil {
				return nil, fmt.Errorf("%q is neither 1, a network such as 172.18.0.0/16, nor an address", s)
			}
			p = netip.PrefixFrom(a, a.BitLen())
		}
		p = p.Masked()
		if !slices.ContainsFunc(privateNetworks, func(n netip.Prefix) bool {
			return n.Bits() <= p.Bits() && n.Contains(p.Addr())
		}) {
			return nil, fmt.Errorf("%s is not a private network (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, fc00::/7)", p)
		}
		proxies = append(proxies, p)
	}
	return proxies, nil
}

// setPublicAddresses fills in the public URL and relay addresses, which
// default to the addresses the server really listens on. webAddr is
// empty when the server serves no browser.
func (c *config) setPublicAddresses(httpAddr, udpAddr, webAddr string) error {
	if c.api.RelayAddr == "" {
		c.api.RelayAddr = udpAddr
	}
	if webAddr != "" {
		public := envOr("DRYWIRE_PUBLIC_WEB_RELAY", webAddr)
		if _, _, err := net.SplitHostPort(public); err != nil {
			return fmt.Errorf("DRYWIRE_PUBLIC_WEB_RELAY: %w", err)
		}
		c.api.WebRelayURL = "https://" + public + web.Path
	}
	public := os.Getenv("DRYWIRE_PUBLIC_URL")
	if public == "" {
		public = "http://" + httpAddr
	}
	var err error
	if c.publicURL, err = auth.CanonicalOrigin(public); err != nil {
		return fmt.Errorf("DRYWIRE_PUBLIC_URL: %w", err)
	}
	if u, _ := url.Parse(c.publicURL); u.Scheme != "https" {
		if ip, err := netip.ParseAddr(u.Hostname()); (err != nil || !ip.IsLoopback()) && u.Hostname() != "localhost" {
			return fmt.Errorf("DRYWIRE_PUBLIC_URL %s: clients only use plain http with this machine; use https://", c.publicURL)
		}
	}
	return nil
}

// mailSender is where sign-in codes go: SMTP, a directory in development,
// or nowhere.
func mailSender() (mail.Sender, error) {
	from := envOr("DRYWIRE_MAIL_FROM", "Drywire <no-reply@drywire.invalid>")
	switch smtpAddr, dir := os.Getenv("DRYWIRE_SMTP_ADDR"), os.Getenv("DRYWIRE_MAIL_DIR"); {
	case smtpAddr != "" && dir != "":
		return nil, errors.New("set DRYWIRE_SMTP_ADDR or DRYWIRE_MAIL_DIR, not both")
	case smtpAddr != "":
		if _, _, err := net.SplitHostPort(smtpAddr); err != nil {
			return nil, fmt.Errorf("DRYWIRE_SMTP_ADDR: %w", err)
		}
		if os.Getenv("DRYWIRE_MAIL_FROM") == "" {
			return nil, errors.New("DRYWIRE_SMTP_ADDR needs DRYWIRE_MAIL_FROM")
		}
		return mail.SMTP{Addr: smtpAddr, Username: os.Getenv("DRYWIRE_SMTP_USER"),
			Password: os.Getenv("DRYWIRE_SMTP_PASSWORD"), From: from}, nil
	case dir != "":
		return mail.Dir{Path: dir, From: from}, nil
	}
	return nil, nil
}
