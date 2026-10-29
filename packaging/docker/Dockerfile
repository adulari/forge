# Forge as a container: `forge serve`, the daemon the desktop and mobile apps connect to.
#
#   docker build -f packaging/docker/Dockerfile -t forge-serve .
#   docker run -d --name forge -p 127.0.0.1:7420:7420 -v forge-home:/home/forge forge-serve
#
# The image installs the released binary with the repository's own install.sh, which checks it
# against the release's SHA-256 manifest. Pin a release with --build-arg FORGE_VERSION=v2.24.1
# (default: the latest). Everything Forge keeps (config, the pairing token, the session store)
# lives in /home/forge, so mount a volume there to keep it across container restarts.
#
# `forge serve --lan` listens on every interface with a self-signed certificate, which is what
# a container needs to be reachable at all. Publish the port on loopback (as above), or put a
# proxy that terminates TLS in front, rather than exposing it to a network you do not control.
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git openssh-client tini \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 --shell /bin/bash forge

ARG FORGE_VERSION=
COPY install.sh /tmp/install.sh
RUN FORGE_VERSION="${FORGE_VERSION}" FORGE_INSTALL_DIR=/usr/local/bin sh /tmp/install.sh \
    && rm /tmp/install.sh \
    && forge --version

USER forge
WORKDIR /home/forge
VOLUME /home/forge
EXPOSE 7420

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD bash -c 'exec 3<>/dev/tcp/127.0.0.1/7420' || exit 1

# tini reaps the shells and language servers agents leave behind, and forwards SIGTERM so the
# daemon drains its running turns before the container stops (docker stop waits 10 s by default;
# raise it with --stop-timeout 60 for long turns).
ENTRYPOINT ["tini", "--", "forge", "serve"]
CMD ["--lan", "--port", "7420"]
