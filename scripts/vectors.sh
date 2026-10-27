#!/bin/sh
# Regenerate the shared protocol test vectors in core/drywire-proto/vectors/
# from the Go generator. Run it after any change to the wire format, then
# commit the new files together with the change.
set -eu

cd "$(dirname "$0")/../server"
go run ./cmd/drywire-vectors -dir ../core/drywire-proto/vectors
