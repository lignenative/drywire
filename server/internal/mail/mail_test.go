package mail

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestADirKeepsEachMessageForItsOwner(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "mail")
	d := Dir{Path: dir, From: "Drywire <no-reply@voice.example.org>"}
	m := Message{To: "alex@example.com", Subject: "482931 est votre code Drywire", Body: "Votre code : 482931\n"}
	if err := d.Send(context.Background(), m); err != nil {
		t.Fatal(err)
	}
	files, _ := os.ReadDir(dir)
	if len(files) != 1 {
		t.Fatalf("%d files", len(files))
	}
	info, _ := files[0].Info()
	if info.Mode().Perm() != 0o600 {
		t.Fatalf("mode %v", info.Mode())
	}
	raw, _ := os.ReadFile(filepath.Join(dir, files[0].Name()))
	text := string(raw)
	for _, want := range []string{"To: alex@example.com\r\n", "Subject: 482931 est votre code Drywire\r\n", "Votre code : 482931\r\n", "@voice.example.org>\r\n"} {
		if !strings.Contains(text, want) {
			t.Errorf("no %q in\n%s", want, text)
		}
	}
}

func TestNoHeaderCanBeSlippedIn(t *testing.T) {
	_, err := format("a@example.org", Message{To: "b@example.org\r\nBcc: c@example.org", Subject: "x"}, time.Now())
	if err != ErrBadAddress {
		t.Fatalf("got %v", err)
	}
	if !strings.HasPrefix(encodeWord("Votre code d’accès"), "=?utf-8?b?") {
		t.Fatal("a subject past ASCII is not encoded")
	}
	if address("Drywire <no-reply@example.org>") != "no-reply@example.org" || address("x@example.org") != "x@example.org" {
		t.Fatal("address")
	}
}
