#!/bin/sh
# List the Makefile targets with the description written after "##".
set -eu

cd "$(dirname "$0")/.."

echo "Usage: make <command>"
echo
grep -E '^[a-z][a-z0-9-]*:.*## ' Makefile |
	sed -E 's/^([a-z0-9-]+):.*## (.*)$/\1\t\2/' |
	while IFS="$(printf '\t')" read -r target description; do
		printf '  %-10s %s\n' "$target" "$description"
	done
