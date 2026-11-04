#!/bin/sh
# Delete the build caches: both Rust workspaces' target directories, where
# cargo keeps every version it ever built and never deletes one. The next
# build makes them again, from scratch, in a few minutes. Nothing else goes:
# no source, no local setting, nothing under .dev/.
set -eu

cd "$(dirname "$0")/.."
PATH="$PATH:/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin"

for dir in core desktop/src-tauri; do
	if [ -d "$dir/target" ]; then
		echo "==> $dir: $(du -sh "$dir/target" | cut -f1) of build cache"
		(cd "$dir" && cargo clean)
	fi
done
