// Command drywire-server runs the Drywire relay and its control plane.
//
// Configuration comes from the environment (see config.go and the server
// README). If an address is taken, the server stops with an error: it never
// moves to another port on its own.
package main

import (
	"context"
	"crypto/tls"
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
	"github.com/lignenative/drywire/server/internal/auth"
	"github.com/lignenative/drywire/server/internal/ledger"
	"github.com/lignenative/drywire/server/internal/relay"
	"github.com/lignenative/drywire/server/internal/store"
)

// version is replaced at build time with -ldflags "-X main.version=…".
var version = "dev"

func main() {
	log := slog.New(slog.NewTextHandler(os.Stderr, nil))
	if err := run(log); err != nil {
		log.Error("drywire-server stopped", "error", err)
		os.Exit(1)
	}
}

func run(log *slog.Logger) error {
	cfg, err := loadConfig()
	if err != nil {
		return err
	}
	for _, w := range cfg.warnings {
		log.Warn(w)
	}

	// The deletion ledger first: it is replayed before anything else
	// reads the database (ADR 0019).
	led, err := ledger.Open(cfg.ledgerPath)
	if err != nil {
		return err
	}
	defer led.Close()
	st, err := store.Open(cfg.dbPath, led, time.Now)
	if err != nil {
		return err
	}
	defer st.Close()
	if err := maintain(context.Background(), st, led, log, true); err != nil {
		return err
	}

	laddr, err := net.ResolveUDPAddr("udp", cfg.udpAddr)
	if err != nil {
		return fmt.Errorf("DRYWIRE_UDP_ADDR: %w", err)
	}
	r := relay.New(time.Now, log)
	udpConn, err := net.ListenUDP("udp", laddr)
	if err != nil {
		return err
	}
	defer udpConn.Close()

	httpListener, err := net.Listen("tcp", cfg.httpAddr)
	if err != nil {
		return err
	}
	// Defaults that depend on the ports really bound (":0" in tests).
	if err := cfg.setPublicAddresses(httpListener.Addr().String(), udpConn.LocalAddr().String()); err != nil {
		return err
	}

	a := auth.New(cfg.publicURL, time.Now)
	server := api.New(cfg.api, st, a, r, log, cfg.accessLog, time.Now)
	httpServer := &http.Server{
		Handler:           server.Handler(),
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       30 * time.Second,
		IdleTimeout:       2 * time.Minute,
		TLSConfig:         &tls.Config{MinVersion: tls.VersionTLS13},
	}
	httpServer.RegisterOnShutdown(server.CloseEvents)

	errs := make(chan error, 2)
	go func() { errs <- r.Serve(udpConn) }()
	go func() {
		if cfg.tlsCert != "" {
			errs <- httpServer.ServeTLS(httpListener, cfg.tlsCert, cfg.tlsKey)
		} else {
			errs <- httpServer.Serve(httpListener)
		}
	}()
	log.Info("drywire-server started", "version", version, "udp", udpConn.LocalAddr().String(),
		"http", httpListener.Addr().String(), "public_url", cfg.publicURL, "tls", cfg.tlsCert != "")

	stop, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	daily := time.NewTicker(24 * time.Hour)
	defer daily.Stop()
	everySecond := time.NewTicker(time.Second)
	defer everySecond.Stop()
	for {
		select {
		case <-stop.Done():
			log.Info("shutting down")
			ctx, done := context.WithTimeout(context.Background(), 5*time.Second)
			defer done()
			return httpServer.Shutdown(ctx)
		case err := <-errs:
			if errors.Is(err, http.ErrServerClosed) {
				return nil
			}
			return err
		case <-everySecond.C:
			server.ExpireSessions()
		case <-daily.C:
			if err := maintain(context.Background(), st, led, log, false); err != nil {
				log.Error("daily maintenance failed", "error", err)
			}
		}
	}
}

// maintain applies the retention rules of spec section 9: at start it
// replays the deletion ledger and prunes it; at start and then daily it
// purges inactive identities and dead invitations.
func maintain(ctx context.Context, st *store.Store, led *ledger.Ledger, log *slog.Logger, starting bool) error {
	if starting {
		entries, err := led.Entries()
		if err != nil {
			return err
		}
		applied, err := st.Replay(ctx, entries)
		if err != nil {
			return fmt.Errorf("replaying the deletion ledger: %w", err)
		}
		if applied > 0 {
			log.Warn("deleted again what the database had brought back (restored backup?)", "deletions", applied)
		}
		if _, err := led.Prune(time.Now().Add(-ledger.Retention)); err != nil {
			return err
		}
	}
	p, err := st.PurgeInactive(ctx)
	if err != nil {
		return err
	}
	if p.Identities > 0 || p.Invitations > 0 {
		log.Info("purged", "inactive_identities", p.Identities, "dead_invitations", p.Invitations)
	}
	return nil
}
