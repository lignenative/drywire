#!/bin/sh
# Run every test suite that exists in the repository. Components are added
# phase by phase, so each one is tested only once its directory is there.
set -eu

cd "$(dirname "$0")/.."

# Homebrew installs rustup keg-only, outside the PATH; cargo installs land
# in ~/.cargo/bin. Appending both is harmless where they do not exist.
PATH="$PATH:/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin"
root=$(pwd)

has_bun() {
	command -v bun >/dev/null 2>&1 || [ -x "$HOME/.bun/bin/bun" ]
}

# A disk that fills during a build fails it halfway, with an error that
# looks like a broken test: stop before, and say what frees space.
free_kb=$(df -Pk "$root" | awk 'NR == 2 { print $4 }')
if [ "$free_kb" -lt $((5 * 1024 * 1024)) ]; then
	echo "drywire: $((free_kb / 1024)) MB free on this disk, and make test needs about 5 GB." >&2
	echo "Free some space first: make clean empties the build caches." >&2
	exit 1
fi

# Cargo keeps every version of every crate it ever built, and never deletes
# one: a workspace's target directory grows with each update, to many times
# what one build needs (0.7 GB for core, 1.7 GB for the desktop app,
# measured from scratch on 2026-11-04).
# Past 6 GB it holds mostly the past, and starting it afresh costs a few
# minutes of building.
bound_target() {
	[ -d "$1/target" ] || return 0
	kb=$(du -sk "$1/target" | cut -f1)
	if [ "$kb" -gt $((6 * 1024 * 1024)) ]; then
		echo "note: $1/target holds $((kb / 1024 / 1024)) GB of past builds; starting it afresh"
		(cd "$1" && cargo clean -q)
	fi
}
bound_target "$root/core"
bound_target "$root/desktop/src-tauri"

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

	# Licenses of the Go dependencies; CI always checks them.
	if command -v go-licenses >/dev/null 2>&1; then
		echo "==> server (go-licenses)"
		"$root/scripts/go-licenses.sh"
	else
		echo "note: Go license check skipped; run: go install github.com/google/go-licenses/v2@v2.0.1"
	fi
fi

if [ -f "$root/core/Cargo.toml" ]; then
	echo "==> core (Rust)"
	cd "$root/core"
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test

	# The Windows backend cannot run here, but it must keep compiling.
	# Without default features libopus is not built, so no Windows C
	# toolchain is needed: `cargo check` links nothing. Only drywire-core's
	# library is checked: the CLI and the tests pull in ring (through
	# rustls), whose C code needs the Windows SDK headers (ADR 0022). CI
	# builds and tests everything on Windows.
	if rustup target list --installed 2>/dev/null | grep -q '^x86_64-pc-windows-msvc$'; then
		echo "==> core (Windows build check)"
		cargo clippy --target x86_64-pc-windows-msvc -p drywire-core \
			--no-default-features --lib -- -D warnings
	else
		echo "note: Windows build check skipped; run: rustup target add x86_64-pc-windows-msvc"
	fi

	# The web client's module (ADR 0040) must keep building for the browser.
	if rustup target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$'; then
		echo "==> core (WebAssembly build check)"
		cargo clippy --target wasm32-unknown-unknown -p drywire-wasm -- -D warnings
	else
		echo "note: WebAssembly build check skipped; run: rustup target add wasm32-unknown-unknown"
	fi

	# Licenses, bans and sources work offline; CI also checks advisories.
	if command -v cargo-deny >/dev/null 2>&1; then
		echo "==> core (cargo deny)"
		cargo deny --log-level error check licenses bans sources
	else
		echo "note: license check skipped; run: cargo install cargo-deny --locked"
	fi
fi

if [ -f "$root/site/i/invite.test.js" ]; then
	echo "==> site (Bun)"
	cd "$root/site"
	if has_bun; then
		PATH="$PATH:$HOME/.bun/bin" bun test
	else
		echo "note: site tests skipped; install Bun (https://bun.sh)"
	fi
fi

if [ -f "$root/scripts/js-licenses.test.ts" ]; then
	echo "==> scripts (Bun)"
	cd "$root"
	if has_bun; then
		PATH="$PATH:$HOME/.bun/bin" bun test ./scripts
	else
		echo "note: scripts tests skipped; install Bun (https://bun.sh)"
	fi
fi

if [ -f "$root/apps/web/package.json" ]; then
	echo "==> web client (Bun)"
	cd "$root/apps/web"
	if ! has_bun; then
		echo "note: web client skipped; install Bun (https://bun.sh)"
	elif ! rustup target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$'; then
		echo "note: web client skipped; run: rustup target add wasm32-unknown-unknown"
	else
		PATH="$PATH:$HOME/.bun/bin"
		# The tests run the module the browser loads, against the vectors.
		(cd "$root/core" && cargo build -q --release -p drywire-wasm --target wasm32-unknown-unknown)
		mkdir -p public
		cp "$root/core/target/wasm32-unknown-unknown/release/drywire_wasm.wasm" public/drywire.wasm
		bun install --frozen-lockfile
		# The desktop interface it imports, from its own node_modules (ADR 0048).
		bun run typecheck
		bun test
		# Errors only: the UI libraries' "use client" directives warn by the hundred.
		bun run build --logLevel error >/dev/null
		cd "$root"
		bun scripts/js-licenses.ts apps/web
	fi
fi

if [ -f "$root/desktop/package.json" ]; then
	echo "==> desktop (Bun)"
	cd "$root/desktop"
	if has_bun; then
		PATH="$PATH:$HOME/.bun/bin"
		bun install --frozen-lockfile
		bun run typecheck
		bun test
		# The licenses of every JavaScript package installed (CLAUDE.md).
		cd "$root"
		bun scripts/js-licenses.ts desktop
	else
		echo "note: desktop interface skipped; install Bun (https://bun.sh)"
	fi

	# The Tauri crate is a workspace of its own (ADR 0030). On Linux it
	# needs the WebKitGTK headers (desktop/README.md).
	cd "$root/desktop/src-tauri"
	if [ "$(uname -s)" = Linux ] && ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
		echo "note: desktop app skipped; install libwebkit2gtk-4.1-dev (desktop/README.md)"
	else
		echo "==> desktop (Rust)"
		cargo fmt --check
		cargo clippy --all-targets -- -D warnings
		cargo test
		if command -v cargo-deny >/dev/null 2>&1; then
			echo "==> desktop (cargo deny)"
			cargo deny --log-level error check licenses bans sources
		fi
	fi
fi
