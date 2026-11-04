package web

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/hex"
	"math/big"
	"net"
	"sync"
	"time"
)

// Browsers accept a certificate they cannot trust through a certificate
// authority when the page names its SHA-256 hash in
// serverCertificateHashes, on three conditions (W3C WebTransport,
// "custom certificate requirements"): an ECDSA P-256 key, a validity of
// two weeks at most, and the current time inside it. The control plane,
// which clients reach over real TLS, hands out the hashes (ADR 0039), so
// the relay needs no certificate from anyone.
const (
	// validity stays under the two weeks browsers allow, with a margin.
	validity = 13 * 24 * time.Hour
	// rotateAfter is how long a certificate is presented. The next one
	// is advertised the whole time, so a client that learned the hashes
	// just before a rotation still connects.
	rotateAfter = 6 * 24 * time.Hour
	// backdate covers clocks a little behind the server's.
	backdate = time.Hour
)

type certificate struct {
	tls  tls.Certificate
	hash string // SHA-256 of the DER encoding, hex
	// since is when it started being presented, or was made for the
	// next one. A next certificate is made when its predecessor starts,
	// so it is presented from day 6 to day 12 of its 13 days.
	since time.Time
}

func newCertificate(now time.Time, hosts []string) (certificate, error) {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return certificate{}, err
	}
	serial, err := rand.Int(rand.Reader, new(big.Int).Lsh(big.NewInt(1), 64))
	if err != nil {
		return certificate{}, err
	}
	template := x509.Certificate{
		SerialNumber: serial,
		Subject:      pkix.Name{CommonName: "drywire relay"},
		NotBefore:    now.Add(-backdate),
		NotAfter:     now.Add(-backdate + validity),
		KeyUsage:     x509.KeyUsageDigitalSignature,
		ExtKeyUsage:  []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
	}
	for _, h := range hosts {
		if ip := net.ParseIP(h); ip != nil {
			template.IPAddresses = append(template.IPAddresses, ip)
		} else if h != "" {
			template.DNSNames = append(template.DNSNames, h)
		}
	}
	der, err := x509.CreateCertificate(rand.Reader, &template, &template, &key.PublicKey, key)
	if err != nil {
		return certificate{}, err
	}
	sum := sha256.Sum256(der)
	return certificate{
		tls:   tls.Certificate{Certificate: [][]byte{der}, PrivateKey: key},
		hash:  hex.EncodeToString(sum[:]),
		since: now,
	}, nil
}

// certificates presents the current certificate and keeps the next one
// ready, so that both hashes can be advertised.
type certificates struct {
	mu            sync.Mutex
	now           func() time.Time
	hosts         []string
	current, next certificate
}

func newCertificates(now func() time.Time, hosts []string) (*certificates, error) {
	c := &certificates{now: now, hosts: hosts}
	var err error
	if c.current, err = newCertificate(now(), hosts); err != nil {
		return nil, err
	}
	if c.next, err = newCertificate(now(), hosts); err != nil {
		return nil, err
	}
	return c, nil
}

// rotate moves to the next certificate once the current one has been
// presented for rotateAfter. The server calls it on every handshake.
func (c *certificates) rotate() error {
	c.mu.Lock()
	defer c.mu.Unlock()
	now := c.now()
	if now.Sub(c.current.since) < rotateAfter {
		return nil
	}
	next, err := newCertificate(now, c.hosts)
	if err != nil {
		return err
	}
	c.current, c.next = c.next, next
	c.current.since = now
	return nil
}

// get is tls.Config.GetCertificate.
func (c *certificates) get(*tls.ClientHelloInfo) (*tls.Certificate, error) {
	if err := c.rotate(); err != nil {
		return nil, err
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	cert := c.current.tls
	return &cert, nil
}

// Hashes are the hashes a client passes to serverCertificateHashes: the
// certificate presented now, and the one that replaces it.
func (c *certificates) Hashes() []string {
	// If making the next certificate fails, the current pair stays valid
	// for a week more, and the next call tries again.
	_ = c.rotate()
	c.mu.Lock()
	defer c.mu.Unlock()
	return []string{c.current.hash, c.next.hash}
}
