// Command drywire-server runs the Drywire relay and its HTTP API.
//
// Configuration comes from the environment:
//
//	DRYWIRE_ROOMS      path of the rooms file (required)
//	DRYWIRE_UDP_ADDR   relay address, default 127.0.0.1:10050
//	DRYWIRE_HTTP_ADDR  HTTP API address, default 127.0.0.1:10050
//
// If an address is taken, the server stops with an error: it never moves to
// another port on its own.
package main

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/lignenative/drywire/server/internal/api"
	"github.com/lignenative/drywire/server/internal/relay"
)

// version is replaced at build time with -ldflags "-X main.version=…".
var version = "dev"

const defaultAddr = "127.0.0.1:10050"

func main() {
	log := slog.New(slog.NewTextHandler(os.Stderr, nil))
	if err := run(log); err != nil {
		log.Error("drywire-server stopped", "error", err)
		os.Exit(1)
	}
}

func run(log *slog.Logger) error {
	roomsPath := os.Getenv("DRYWIRE_ROOMS")
	if roomsPath == "" {
		return errors.New("DRYWIRE_ROOMS is not set: point it to a rooms file (see server/rooms.dev.json)")
	}
	udpAddr := envOr("DRYWIRE_UDP_ADDR", defaultAddr)
	httpAddr := envOr("DRYWIRE_HTTP_ADDR", defaultAddr)

	rooms, err := relay.LoadRooms(roomsPath)
	if err != nil {
		return err
	}

	laddr, err := net.ResolveUDPAddr("udp", udpAddr)
	if err != nil {
		return fmt.Errorf("DRYWIRE_UDP_ADDR: %w", err)
	}
	if rooms.DevOnly && !laddr.IP.IsLoopback() {
		return fmt.Errorf("%s holds public development secrets and may only be served on a loopback address, not %s", roomsPath, udpAddr)
	}
	udpConn, err := net.ListenUDP("udp", laddr)
	if err != nil {
		return err
	}
	defer udpConn.Close()

	httpListener, err := net.Listen("tcp", httpAddr)
	if err != nil {
		return err
	}

	started := time.Now()
	r := relay.New(rooms, time.Now, log)
	httpServer := &http.Server{
		Handler:           api.Handler(r, version, started),
		ReadHeaderTimeout: 5 * time.Second,
	}

	errs := make(chan error, 2)
	go func() { errs <- r.Serve(udpConn) }()
	go func() { errs <- httpServer.Serve(httpListener) }()
	log.Info("drywire-server started", "version", version, "udp", udpConn.LocalAddr().String(),
		"http", httpListener.Addr().String(), "rooms", len(rooms.Rooms))

	stop, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	select {
	case <-stop.Done():
		log.Info("shutting down")
	case err := <-errs:
		return err
	}

	ctx, done := context.WithTimeout(context.Background(), 5*time.Second)
	defer done()
	return httpServer.Shutdown(ctx)
}

func envOr(name, fallback string) string {
	if v := os.Getenv(name); v != "" {
		return v
	}
	return fallback
}
