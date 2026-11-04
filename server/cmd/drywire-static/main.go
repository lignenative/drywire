// Command drywire-static serves the web client's files, and nothing else.
//
// It is the front of a deployed instance (ADR 0049): a service of its own
// beside drywire-server, which stays an API only. The reverse proxy sends
// /v1/… to the control plane and every other path here, so the page and
// the control plane share one origin and no cross-origin request is ever
// made. It holds no state, reads no request body and keeps no log of who
// asked for what.
//
//	drywire-static           serve
//	drywire-static health    exit 0 if it answers (the image's health check)
//
// Environment:
//
//	DRYWIRE_STATIC_ADDR  TCP address to listen on, default 127.0.0.1:10052
//	DRYWIRE_STATIC_DIR   the web client's build (apps/web/dist), default "web"
package main

import (
	"context"
	"errors"
	"fmt"
	"io/fs"
	"log"
	"net"
	"net/http"
	"net/netip"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"
)

const defaultAddr = "127.0.0.1:10052"

func main() {
	log.SetFlags(0)
	log.SetPrefix("drywire-static: ")
	var err error
	switch {
	case len(os.Args) == 1:
		err = serve()
	case len(os.Args) == 2 && os.Args[1] == "health":
		err = health()
	default:
		err = errors.New("usage: drywire-static [health]")
	}
	if err != nil {
		log.Fatal(err)
	}
}

func serve() error {
	addr := envOr("DRYWIRE_STATIC_ADDR", defaultAddr)
	dir := envOr("DRYWIRE_STATIC_DIR", "web")
	// A missing build is a deployment mistake: say so at start, not as a
	// 404 to every visitor.
	if _, err := os.Stat(dir + "/index.html"); err != nil {
		return fmt.Errorf("DRYWIRE_STATIC_DIR: no web client to serve: %w", err)
	}
	ln, err := net.Listen("tcp", addr)
	if err != nil {
		return err
	}
	srv := &http.Server{
		Handler:           handler(os.DirFS(dir)),
		ReadHeaderTimeout: 10 * time.Second,
		IdleTimeout:       2 * time.Minute,
		// Request errors (a client gone mid-file) say nothing worth keeping.
		ErrorLog: log.New(discard{}, "", 0),
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	go func() {
		<-ctx.Done()
		shutdown, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		srv.Shutdown(shutdown)
	}()
	log.Printf("serving %s on %s", dir, ln.Addr())
	if err := srv.Serve(ln); !errors.Is(err, http.ErrServerClosed) {
		return err
	}
	return nil
}

// handler serves files from files: the page at "/", and the files its
// build made. No directory is ever listed.
func handler(files fs.FS) http.Handler {
	fileServer := http.FileServerFS(files)
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		h := w.Header()
		h.Set("X-Content-Type-Options", "nosniff")
		// The page is never framed by another site, and never tells the
		// sites it links to where the visitor came from.
		h.Set("Content-Security-Policy", "frame-ancestors 'none'")
		h.Set("Referrer-Policy", "no-referrer")
		if r.Method != http.MethodGet && r.Method != http.MethodHead {
			h.Set("Allow", "GET, HEAD")
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		if strings.HasSuffix(r.URL.Path, "/") && r.URL.Path != "/" {
			http.NotFound(w, r)
			return
		}
		// Vite names what it builds in assets/ after its content: a new
		// version is a new name, so those never need asking again. The
		// rest (the page, the WebAssembly module) keeps its name across
		// versions, so the browser checks it on every load and a deploy
		// shows at the next reload.
		if strings.HasPrefix(r.URL.Path, "/assets/") {
			h.Set("Cache-Control", "public, max-age=31536000, immutable")
		} else {
			h.Set("Cache-Control", "no-cache")
		}
		fileServer.ServeHTTP(w, r)
	})
}

// health asks the running server for its page.
func health() error {
	addr := envOr("DRYWIRE_STATIC_ADDR", defaultAddr)
	host, port, err := net.SplitHostPort(addr)
	if err != nil {
		return fmt.Errorf("DRYWIRE_STATIC_ADDR: %w", err)
	}
	// A server listening on every address answers on loopback too.
	if ip, err := netip.ParseAddr(host); host == "" || (err == nil && ip.IsUnspecified()) {
		host = "127.0.0.1"
	}
	client := &http.Client{Timeout: 3 * time.Second}
	resp, err := client.Head("http://" + net.JoinHostPort(host, port) + "/")
	if err != nil {
		return err
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("/ answered %s", resp.Status)
	}
	return nil
}

func envOr(name, fallback string) string {
	if v := os.Getenv(name); v != "" {
		return v
	}
	return fallback
}

type discard struct{}

func (discard) Write(p []byte) (int, error) { return len(p), nil }
