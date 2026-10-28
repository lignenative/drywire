# The release image of drywire-server: the static binary and nothing else.
#
# The binary is not built here. scripts/deploy.sh cross-compiles it on the
# operator's machine (CGO_ENABLED=0, so it needs no C library at all) and
# sends it to the host beside this file. Starting from scratch, building
# the image downloads nothing: the host never fetches code or images to
# run Drywire.
FROM scratch

COPY --chmod=0555 drywire-server /drywire-server

# An unprivileged user of its own. Nothing in the image runs as root, and
# the binary can only write to /data.
USER 10050:10050

# The database and the deletion ledger live in the one volume mounted on
# /data. Created after USER, the directory belongs to that user, and so
# does a new volume Docker fills from it.
WORKDIR /data
ENV DRYWIRE_DB=/data/drywire.db \
    DRYWIRE_LEDGER=/data/drywire.deletions
VOLUME /data

# Without arguments the server runs; `health`, `backup` and `restore` are
# the maintenance commands (ADR 0025).
ENTRYPOINT ["/drywire-server"]
