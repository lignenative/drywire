#!/bin/sh
# Run every test suite that exists in the repository. Components are added
# phase by phase, so each one is tested only once its directory is there.
set -eu

cd "$(dirname "$0")/.."

# Homebrew installs rustup keg-only, outside the PATH; cargo installs land
# in ~/.cargo/bin. Appending both is harmless where they do not exist.
PATH="$PATH:/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin"
root=$(pwd)

if [ -f server/go.mod ]; then
	echo "==> server (Go)"
	cd "$root/server"
	unformatted=$(gofmt -l .)
	if [ -n "$unformatted" ]; then
		echo "These files need gofmt (run make fmt):" >&2
		echo "$unformatted" >&2
		exit 1
	fi
	go vet ./...
	go test -race ./...
fi

if [ -f "$root/core/Cargo.toml" ]; then
	echo "==> core (Rust)"
	cd "$root/core"
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test

	# The Windows backend cannot run here, but it must keep compiling.
	# Without default features libopus is not built, so no Windows C
	# toolchain is needed: `cargo check` links nothing.
	if rustup target list --installed 2>/dev/null | grep -q '^x86_64-pc-windows-msvc$'; then
		echo "==> core (Windows build check)"
		cargo clippy --target x86_64-pc-windows-msvc -p drywire-core -p drywire-cli \
			--no-default-features --all-targets -- -D warnings
	else
		echo "note: Windows build check skipped; run: rustup target add x86_64-pc-windows-msvc"
	fi

	# Licenses, bans and sources work offline; CI also checks advisories.
	if command -v cargo-deny >/dev/null 2>&1; then
		echo "==> core (cargo deny)"
		cargo deny --log-level error check licenses bans sources
	else
		echo "note: license check skipped; run: cargo install cargo-deny --locked"
	fi
fi
