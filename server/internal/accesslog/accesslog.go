// Package accesslog keeps the access log in a directory of daily files and
// deletes them once they are older than the retention (docs/privacy.md,
// processing 3: one year at most).
//
// The log then survives what Docker's own logs do not: the container being
// recreated on every deploy. An incident can be dated months later, and
// nothing is kept longer than the privacy notice says, without any outside
// rotation tool.
//
// Files are named access-YYYY-MM-DD.jsonl, by UTC day, readable by the
// server's user only. A file is deleted once every line in it is older
// than the retention: on the day after its last day plus the retention.
package accesslog

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

// MaxDays is the longest retention allowed: the privacy notice promises a
// year at most.
const MaxDays = 365

const (
	prefix = "access-"
	suffix = ".jsonl"
	layout = "2006-01-02"
)

// Dir is an io.Writer that appends to the day's file, opening the next one
// at midnight UTC and deleting those past the retention. Each Write is one
// log line; it is safe for concurrent use.
type Dir struct {
	dir  string
	days int
	now  func() time.Time

	mu   sync.Mutex
	day  string
	file *os.File
}

// Open prepares dir (created if missing) for a log kept `days` days, at
// most MaxDays, and deletes the files already past that.
func Open(dir string, days int, now func() time.Time) (*Dir, error) {
	if days < 1 || days > MaxDays {
		return nil, fmt.Errorf("the access log is kept 1 to %d days, not %d", MaxDays, days)
	}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, err
	}
	d := &Dir{dir: dir, days: days, now: now}
	d.mu.Lock()
	defer d.mu.Unlock()
	if err := d.prune(now()); err != nil {
		return nil, err
	}
	return d, nil
}

// Write appends p to the current day's file.
func (d *Dir) Write(p []byte) (int, error) {
	d.mu.Lock()
	defer d.mu.Unlock()
	now := d.now()
	day := now.UTC().Format(layout)
	if day != d.day || d.file == nil {
		if d.file != nil {
			_ = d.file.Close()
			d.file = nil
		}
		f, err := os.OpenFile(filepath.Join(d.dir, prefix+day+suffix),
			os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
		if err != nil {
			return 0, err
		}
		d.file, d.day = f, day
		// A new day is when the oldest file may have expired.
		if err := d.prune(now); err != nil {
			return 0, err
		}
	}
	return d.file.Write(p)
}

// Close closes the current file.
func (d *Dir) Close() error {
	d.mu.Lock()
	defer d.mu.Unlock()
	if d.file == nil {
		return nil
	}
	err := d.file.Close()
	d.file = nil
	return err
}

// prune deletes the files whose day ended more than `days` days before
// now. Files that do not look like the log's are left alone.
func (d *Dir) prune(now time.Time) error {
	entries, err := os.ReadDir(d.dir)
	if err != nil {
		return err
	}
	for _, e := range entries {
		name := e.Name()
		if e.IsDir() || !strings.HasPrefix(name, prefix) || !strings.HasSuffix(name, suffix) {
			continue
		}
		day, err := time.Parse(layout, strings.TrimSuffix(strings.TrimPrefix(name, prefix), suffix))
		if err != nil {
			continue
		}
		// Its last line is at most the end of that day.
		if now.Sub(day.AddDate(0, 0, 1)) > time.Duration(d.days)*24*time.Hour {
			if err := os.Remove(filepath.Join(d.dir, name)); err != nil && !os.IsNotExist(err) {
				return err
			}
		}
	}
	return nil
}
