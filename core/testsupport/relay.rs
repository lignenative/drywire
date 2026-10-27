//! Starts the real Go relay for a Rust test.
//!
//! Shared by the integration tests of several crates with
//! `#[path = "../../testsupport/relay.rs"] mod relay;`, because the relay is
//! a separate program, not a crate anyone can depend on.
//!
//! The relay is built from `server/` with `go build` once per test binary,
//! or taken from `DRYWIRE_SERVER_BIN` if that variable names a binary. It
//! listens on ports the system picks, so tests run in parallel, and serves
//! the development rooms of `server/rooms.dev.json`. These tests need Go:
//! without it they fail and say so, rather than pass without testing
//! anything.

#![allow(dead_code)] // each test binary uses a different part

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

/// The repository root, from this crate's manifest directory.
pub fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while !dir.join("server").join("go.mod").exists() {
        assert!(
            dir.pop(),
            "no server/go.mod above {}",
            env!("CARGO_MANIFEST_DIR")
        );
    }
    dir
}

/// The session secrets of `server/rooms.dev.json`: session `n` of the
/// development rooms has the secret made of 32 bytes `0xnn`.
pub fn dev_secret(session: u32) -> [u8; 32] {
    [session as u8 * 0x11; 32]
}

fn server_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        if let Some(path) = std::env::var_os("DRYWIRE_SERVER_BIN") {
            return PathBuf::from(path);
        }
        // One binary per test process, so parallel test binaries never
        // write the same file.
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!(
                "drywire-server-{}{}",
                std::process::id(),
                std::env::consts::EXE_SUFFIX
            ));
        let status = Command::new("go")
            .args(["build", "-o"])
            .arg(&out)
            .arg("./cmd/drywire-server")
            .current_dir(repo_root().join("server"))
            .status()
            .expect("these tests start the real relay and need Go: install it, or set DRYWIRE_SERVER_BIN");
        assert!(status.success(), "go build of the relay failed");
        out
    })
}

/// Numbers each relay a test process starts.
static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// A running relay, stopped when dropped.
pub struct Relay {
    child: Child,
    /// Where it receives UDP.
    pub udp: SocketAddr,
    /// Where its HTTP API listens.
    pub http: SocketAddr,
}

impl Relay {
    /// Starts a relay with the development rooms.
    pub fn start() -> Relay {
        let rooms = repo_root().join("server").join("rooms.dev.json");
        // A database of its own in a fresh directory, so tests never share
        // or leave state.
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "relay-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the relay's directory");
        let mut child = Command::new(server_binary())
            .env("DRYWIRE_ROOMS", rooms)
            .env("DRYWIRE_DB", dir.join("drywire.db"))
            .env("DRYWIRE_ACCESS_LOG", "off")
            .env("DRYWIRE_UDP_ADDR", "127.0.0.1:0")
            .env("DRYWIRE_HTTP_ADDR", "127.0.0.1:0")
            .stderr(Stdio::piped())
            .spawn()
            .expect("start drywire-server");

        // The server logs the addresses it got in its "started" line.
        let mut lines = BufReader::new(child.stderr.take().expect("piped"));
        let mut line = String::new();
        let (udp, http) = loop {
            line.clear();
            let n = lines.read_line(&mut line).expect("read the relay's log");
            assert!(n > 0, "drywire-server exited before it started");
            if line.contains("drywire-server started") {
                break (field(&line, "udp"), field(&line, "http"));
            }
        };
        // Keep reading its log, so the pipe never fills and blocks it.
        std::thread::spawn(move || {
            for line in lines.lines().map_while(Result::ok) {
                eprintln!("relay: {line}");
            }
        });
        Relay { child, udp, http }
    }

    /// `GET /v1/status`, parsed.
    pub fn status(&self) -> serde_json::Value {
        let mut stream = TcpStream::connect(self.http).expect("connect to the relay's API");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(
            stream,
            "GET /v1/status HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let body = response.split_once("\r\n\r\n").expect("an HTTP response").1;
        serde_json::from_str(body).expect("JSON status")
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The address after `name=` in a log line.
fn field(line: &str, name: &str) -> SocketAddr {
    let start = line
        .find(&format!(" {name}="))
        .unwrap_or_else(|| panic!("no {name}= in {line:?}"))
        + name.len()
        + 2;
    line[start..]
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap_or_else(|e| panic!("{name} in {line:?}: {e}"))
}
