package mail

import (
	"bufio"
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/base64"
	"math/big"
	"net"
	"strings"
	"testing"
	"time"
)

// relay is a submission server as eusend's (smtp.eusend.dev:465): TLS
// from the first byte, a greeting the client must wait for, AUTH PLAIN
// with a fixed user name and the API key as password.
type relay struct {
	addr   string
	roots  *x509.CertPool
	auth   chan string
	data   chan string
	sender chan string
}

func startRelay(t *testing.T) *relay {
	t.Helper()
	key, _ := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	template := &x509.Certificate{
		SerialNumber:          big.NewInt(1),
		Subject:               pkix.Name{CommonName: "smtp.relay.test"},
		DNSNames:              []string{"localhost"},
		NotBefore:             time.Now().Add(-time.Hour),
		NotAfter:              time.Now().Add(time.Hour),
		IsCA:                  true,
		KeyUsage:              x509.KeyUsageDigitalSignature | x509.KeyUsageCertSign,
		ExtKeyUsage:           []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		BasicConstraintsValid: true,
	}
	der, err := x509.CreateCertificate(rand.Reader, template, template, &key.PublicKey, key)
	if err != nil {
		t.Fatal(err)
	}
	cert, _ := x509.ParseCertificate(der)
	roots := x509.NewCertPool()
	roots.AddCert(cert)
	ln, err := tls.Listen("tcp", "127.0.0.1:0", &tls.Config{
		Certificates: []tls.Certificate{{Certificate: [][]byte{der}, PrivateKey: key}},
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { ln.Close() })
	r := &relay{addr: ln.Addr().String(), roots: roots, auth: make(chan string, 1), data: make(chan string, 1), sender: make(chan string, 1)}
	go func() {
		conn, err := ln.Accept()
		if err != nil {
			return
		}
		defer conn.Close()
		in := bufio.NewReader(conn)
		say := func(line string) { conn.Write([]byte(line + "\r\n")) }
		say("220 relay.test ESMTP")
		for {
			line, err := in.ReadString('\n')
			if err != nil {
				return
			}
			line = strings.TrimRight(line, "\r\n")
			switch verb := strings.ToUpper(strings.Fields(line + " x")[0]); verb {
			case "EHLO":
				say("250-relay.test")
				say("250-AUTH PLAIN LOGIN")
				say("250 8BITMIME")
			case "AUTH":
				plain, _ := base64.StdEncoding.DecodeString(strings.Fields(line)[2])
				r.auth <- string(plain)
				say("235 2.7.0 accepted")
			case "MAIL":
				r.sender <- line
				say("250 ok")
			case "RCPT":
				say("250 ok")
			case "DATA":
				say("354 go on")
				var body strings.Builder
				for {
					l, err := in.ReadString('\n')
					if err != nil || l == ".\r\n" {
						break
					}
					body.WriteString(l)
				}
				r.data <- body.String()
				say("250 queued")
			case "QUIT":
				say("221 bye")
				return
			default:
				say("502 no")
			}
		}
	}()
	return r
}

func TestACodeGoesThroughARelayOverImplicitTLS(t *testing.T) {
	r := startRelay(t)
	_, port, _ := net.SplitHostPort(r.addr)
	s := SMTP{
		Addr:        "localhost:" + port,
		Username:    "eusend",
		Password:    "eu_test_example",
		From:        "Drywire <no-reply@voice.example.org>",
		implicitTLS: true,
		roots:       r.roots,
	}
	m := Message{To: "alex@example.com", Subject: "482931 est votre code Drywire", Body: "Votre code : 482931\n"}
	if err := s.Send(context.Background(), m); err != nil {
		t.Fatal(err)
	}
	if auth := <-r.auth; auth != "\x00eusend\x00eu_test_example" {
		t.Fatalf("auth %q", auth)
	}
	if sender := <-r.sender; !strings.Contains(sender, "<no-reply@voice.example.org>") {
		t.Fatalf("sender %q", sender)
	}
	data := <-r.data
	for _, want := range []string{"Subject: 482931 est votre code Drywire\r\n", "Votre code : 482931\r\n", "To: alex@example.com\r\n"} {
		if !strings.Contains(data, want) {
			t.Errorf("no %q in\n%s", want, data)
		}
	}
}
