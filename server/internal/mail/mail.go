// Package mail sends the instance's few e-mails: the sign-in codes of ADR
// 0055. Standard library only: SMTP for a real instance, a directory of
// files in development, where nothing must leave the machine.
//
// A message's content is a code: nothing here logs it, and callers must
// not either.
package mail

import (
	"context"
	"crypto/rand"
	"crypto/tls"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"fmt"
	"net"
	"net/smtp"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// Message is one plain-text e-mail.
type Message struct {
	To      string
	Subject string
	Body    string
}

// Sender sends messages.
type Sender interface {
	Send(ctx context.Context, m Message) error
}

// ErrBadAddress is an address no message is sent to: one that could add
// a header line.
var ErrBadAddress = errors.New("mail: not an address to send to")

// format writes m as an RFC 5322 message, UTF-8, from `from`.
func format(from string, m Message, now time.Time) ([]byte, error) {
	for _, field := range []string{from, m.To, m.Subject} {
		if strings.ContainsAny(field, "\r\n") {
			return nil, ErrBadAddress
		}
	}
	var id [12]byte
	rand.Read(id[:])
	domain := "drywire.invalid"
	if at := strings.LastIndexByte(from, '@'); at >= 0 {
		domain = strings.Trim(from[at+1:], "<> ")
	}
	var b strings.Builder
	fmt.Fprintf(&b, "From: %s\r\n", from)
	fmt.Fprintf(&b, "To: %s\r\n", m.To)
	// RFC 2047: a subject in UTF-8 rides as base64 words.
	fmt.Fprintf(&b, "Subject: %s\r\n", encodeWord(m.Subject))
	fmt.Fprintf(&b, "Date: %s\r\n", now.UTC().Format(time.RFC1123Z))
	fmt.Fprintf(&b, "Message-ID: <%s@%s>\r\n", hex.EncodeToString(id[:]), domain)
	b.WriteString("MIME-Version: 1.0\r\n")
	b.WriteString("Content-Type: text/plain; charset=utf-8\r\n")
	b.WriteString("Content-Transfer-Encoding: 8bit\r\n\r\n")
	b.WriteString(strings.ReplaceAll(m.Body, "\n", "\r\n"))
	return []byte(b.String()), nil
}

func encodeWord(s string) string {
	for _, r := range s {
		if r > 126 {
			return "=?utf-8?b?" + base64.StdEncoding.EncodeToString([]byte(s)) + "?="
		}
	}
	return s
}

// Dir writes each message to a file in Path, for development: the code is
// read there, and no mail leaves the machine.
type Dir struct {
	Path string
	From string
}

// Send writes m to <Path>/<time>-<random>.eml, readable by its owner only.
func (d Dir) Send(_ context.Context, m Message) error {
	now := time.Now()
	raw, err := format(d.From, m, now)
	if err != nil {
		return err
	}
	if err := os.MkdirAll(d.Path, 0o700); err != nil {
		return err
	}
	var suffix [4]byte
	rand.Read(suffix[:])
	// To the nanosecond, so that the names sort as the mails were sent:
	// the newest code for an address is the last file.
	name := fmt.Sprintf("%s-%s.eml", now.UTC().Format("20060102T150405.000000000"), hex.EncodeToString(suffix[:]))
	return os.WriteFile(filepath.Join(d.Path, name), raw, 0o600)
}

// SMTP sends through a mail provider's submission port, always over TLS:
// implicit on port 465, STARTTLS anywhere else.
type SMTP struct {
	Addr     string // host:port
	Username string
	Password string
	From     string
}

// Send delivers m, giving up after 15 seconds.
func (s SMTP) Send(ctx context.Context, m Message) error {
	raw, err := format(s.From, m, time.Now())
	if err != nil {
		return err
	}
	host, port, err := net.SplitHostPort(s.Addr)
	if err != nil {
		return err
	}
	ctx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	dialer := &net.Dialer{}
	tlsConfig := &tls.Config{ServerName: host, MinVersion: tls.VersionTLS12}
	var conn net.Conn
	if port == "465" {
		conn, err = (&tls.Dialer{NetDialer: dialer, Config: tlsConfig}).DialContext(ctx, "tcp", s.Addr)
	} else {
		conn, err = dialer.DialContext(ctx, "tcp", s.Addr)
	}
	if err != nil {
		return err
	}
	deadline, _ := ctx.Deadline()
	conn.SetDeadline(deadline)
	c, err := smtp.NewClient(conn, host)
	if err != nil {
		conn.Close()
		return err
	}
	defer c.Close()
	if port != "465" {
		if ok, _ := c.Extension("STARTTLS"); !ok {
			return errors.New("mail: the server offers no STARTTLS")
		}
		if err := c.StartTLS(tlsConfig); err != nil {
			return err
		}
	}
	if s.Username != "" {
		if err := c.Auth(smtp.PlainAuth("", s.Username, s.Password, host)); err != nil {
			return err
		}
	}
	if err := c.Mail(address(s.From)); err != nil {
		return err
	}
	if err := c.Rcpt(m.To); err != nil {
		return err
	}
	w, err := c.Data()
	if err != nil {
		return err
	}
	if _, err := w.Write(raw); err != nil {
		return err
	}
	if err := w.Close(); err != nil {
		return err
	}
	return c.Quit()
}

// address is the bare address of `Name <user@host>`.
func address(from string) string {
	if i, j := strings.LastIndexByte(from, '<'), strings.LastIndexByte(from, '>'); i >= 0 && j > i {
		return from[i+1 : j]
	}
	return from
}
