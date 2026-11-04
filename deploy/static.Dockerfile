# The web client's image: drywire-static and the client's build, nothing
# else (ADR 0049).
#
# As with the server's image, nothing is built or downloaded here:
# scripts/deploy.sh cross-compiles drywire-static and builds apps/web on
# the operator's machine, then sends both beside this file.
FROM scratch

COPY --chmod=0555 drywire-static /drywire-static
# Owned by root, so the user below can read the files but never change
# them; the container is read-only besides.
COPY web /web
WORKDIR /web
ENV DRYWIRE_STATIC_DIR=/web

USER 10052:10052

# Without arguments it serves; `health` is the image's health check.
ENTRYPOINT ["/drywire-static"]
