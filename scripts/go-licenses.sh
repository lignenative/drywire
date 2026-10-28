#!/bin/sh
# Check the licenses of every Go dependency of the server against the
# list CLAUDE.md allows. Run by make test when go-licenses is installed,
# and by CI always:
#   go install github.com/google/go-licenses/v2@v2.0.1
#
# The server's own packages are skipped (their license, MIT, is at the
# repository root, above the Go module); what they import is checked.
set -eu

cd "$(dirname "$0")/../server"

# Release builds are for Linux: check what goes into them.
status=0
out=$(GOOS=linux go-licenses check ./... --include_tests \
	--ignore github.com/lignenative/drywire \
	--allowed_licenses=MIT,BSD-2-Clause,BSD-3-Clause,Apache-2.0,ISC,Zlib,MPL-2.0 \
	2>&1) || status=$?
# go-licenses warns that golang.org/x/sys has assembly files it cannot
# read for imports; they import nothing. Everything else is shown.
printf '%s\n' "$out" | grep -v -e 'contains non-Go code' -e '\.s$' >&2 || true
exit "$status"
