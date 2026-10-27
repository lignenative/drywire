// Command drywire-vectors writes the shared protocol test vectors.
//
// Run it with `make vectors` after changing the wire format. A test in
// internal/vectors fails while the files on disk differ from what this
// command would write.
package main

import (
	"flag"
	"fmt"
	"os"

	"github.com/lignenative/drywire/server/internal/vectors"
)

func main() {
	dir := flag.String("dir", "../core/drywire-proto/vectors", "directory to write the vectors to")
	flag.Parse()

	m, files := vectors.Build()
	if err := vectors.Write(*dir, m, files); err != nil {
		fmt.Fprintln(os.Stderr, "drywire-vectors:", err)
		os.Exit(1)
	}
	fmt.Printf("wrote %d vectors to %s\n", len(m.Vectors), *dir)
}
