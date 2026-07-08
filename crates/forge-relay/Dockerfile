# Multi-stage build for the forge-relay binary (ADR-0012). No existing Dockerfile precedent in
# this repo — this is the first. Built from the WORKSPACE root (fly.toml points `context` there)
# so the workspace Cargo.lock and forge-relay's path stay consistent with a normal `cargo build`.

FROM rust:1.85-slim-bookworm AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release -p forge-relay --locked

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/forge-relay /usr/local/bin/forge-relay
EXPOSE 8787
ENTRYPOINT ["/usr/local/bin/forge-relay"]
