package ledger

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestEntriesSurviveAReopenAndPruneKeepsTheRecentOnes(t *testing.T) {
	path := filepath.Join(t.TempDir(), "deletions.jsonl")
	l, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	for at := int64(100); at <= 300; at += 100 {
		if err := l.Append(Entry{At: at, Kind: KindSpace, Space: "00"}); err != nil {
			t.Fatal(err)
		}
	}
	l.Close()

	if info, err := os.Stat(path); err != nil || info.Mode().Perm() != 0o600 {
		t.Fatalf("ledger mode %v, %v", info.Mode(), err)
	}
	l, err = Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer l.Close()
	removed, err := l.Prune(time.Unix(200, 0))
	if err != nil || removed != 1 {
		t.Fatalf("pruned %d, %v", removed, err)
	}
	// Appending after a prune goes to the new file.
	if err := l.Append(Entry{At: 400, Kind: KindIdentity, Identity: KeyHash([]byte{1})}); err != nil {
		t.Fatal(err)
	}
	entries, err := l.Entries()
	if err != nil {
		t.Fatal(err)
	}
	var ats []int64
	for _, e := range entries {
		ats = append(ats, e.At)
	}
	if len(ats) != 3 || ats[0] != 200 || ats[2] != 400 {
		t.Fatalf("entries at %v", ats)
	}
	if info, _ := os.Stat(path); info.Mode().Perm() != 0o600 {
		t.Fatalf("pruned ledger mode %v", info.Mode())
	}
}

func TestALineCutByACrashIsSkippedAndOtherDamageReported(t *testing.T) {
	path := filepath.Join(t.TempDir(), "deletions.jsonl")
	os.WriteFile(path, []byte("{\"at\":1,\"kind\":\"space\"}\n{\"at\":"), 0o600)
	l, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer l.Close()
	if entries, err := l.Entries(); err != nil || len(entries) != 1 {
		t.Fatalf("a last line cut short: %v, %v", entries, err)
	}
	os.WriteFile(path, []byte("{\"at\":\n{\"at\":1,\"kind\":\"space\"}\n"), 0o600)
	if _, err := l.Entries(); err == nil {
		t.Fatal("a damaged line in the middle was accepted")
	}
}
