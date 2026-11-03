package accesslog

import (
	"os"
	"path/filepath"
	"slices"
	"testing"
	"time"
)

func names(t *testing.T, dir string) []string {
	t.Helper()
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	var out []string
	for _, e := range entries {
		out = append(out, e.Name())
	}
	return out
}

func TestOneFilePerDayAndNothingPastTheRetention(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "access")
	now := time.Date(2026, 11, 4, 23, 59, 0, 0, time.UTC)
	clock := func() time.Time { return now }
	d, err := Open(dir, 2, clock)
	if err != nil {
		t.Fatal(err)
	}
	defer d.Close()

	write := func(line string) {
		t.Helper()
		if _, err := d.Write([]byte(line + "\n")); err != nil {
			t.Fatal(err)
		}
	}
	write("a")
	now = now.Add(2 * time.Minute) // past midnight UTC
	write("b")
	write("c")
	if got, want := names(t, dir), []string{"access-2026-11-04.jsonl", "access-2026-11-05.jsonl"}; !slices.Equal(got, want) {
		t.Fatalf("files %v, want %v", got, want)
	}
	data, _ := os.ReadFile(filepath.Join(dir, "access-2026-11-05.jsonl"))
	if string(data) != "b\nc\n" {
		t.Fatalf("second day holds %q", data)
	}
	info, _ := os.Stat(filepath.Join(dir, "access-2026-11-05.jsonl"))
	if info.Mode().Perm() != 0o600 {
		t.Fatalf("mode %v: readable by the server's user only", info.Mode())
	}

	// A line written at the very start of the 4th would be two days old
	// at the start of the 6th: the file is kept until then, not after.
	now = time.Date(2026, 11, 5, 23, 59, 0, 0, time.UTC)
	write("d")
	if !slices.Contains(names(t, dir), "access-2026-11-04.jsonl") {
		t.Fatal("deleted before its retention ran out")
	}
	now = time.Date(2026, 11, 6, 0, 1, 0, 0, time.UTC)
	write("e")
	if got := names(t, dir); slices.Contains(got, "access-2026-11-04.jsonl") {
		t.Fatalf("kept past its retention: %v", got)
	}
}

func TestOpeningDeletesOldFilesAndLeavesOthersAlone(t *testing.T) {
	dir := t.TempDir()
	for _, name := range []string{"access-2025-01-01.jsonl", "access-2026-11-03.jsonl", "notes.txt", "access-bad.jsonl"} {
		if err := os.WriteFile(filepath.Join(dir, name), nil, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	now := time.Date(2026, 11, 4, 12, 0, 0, 0, time.UTC)
	d, err := Open(dir, MaxDays, func() time.Time { return now })
	if err != nil {
		t.Fatal(err)
	}
	defer d.Close()
	got := names(t, dir)
	want := []string{"access-2026-11-03.jsonl", "access-bad.jsonl", "notes.txt"}
	if !slices.Equal(got, want) {
		t.Fatalf("files %v, want %v", got, want)
	}
}

func TestAFileThatCannotBeDeletedCostsNoLine(t *testing.T) {
	dir := t.TempDir()
	// An expired "file" that is a non-empty directory: Remove fails.
	stuck := filepath.Join(dir, "access-2020-01-01.jsonl")
	if err := os.MkdirAll(filepath.Join(stuck, "inside"), 0o700); err != nil {
		t.Fatal(err)
	}
	now := time.Date(2026, 11, 4, 12, 0, 0, 0, time.UTC)
	d := &Dir{dir: dir, days: 30, now: func() time.Time { return now }}
	if _, err := d.Write([]byte("kept\n")); err != nil {
		t.Fatalf("the line was refused: %v", err)
	}
	defer d.Close()
	data, _ := os.ReadFile(filepath.Join(dir, "access-2026-11-04.jsonl"))
	if string(data) != "kept\n" {
		t.Fatalf("today's file holds %q", data)
	}
}

func TestRetentionIsBounded(t *testing.T) {
	for _, days := range []int{0, -1, MaxDays + 1} {
		if _, err := Open(t.TempDir(), days, time.Now); err == nil {
			t.Fatalf("%d days accepted", days)
		}
	}
}
