# syntax=docker/dockerfile:1
FROM rust:1-bookworm AS chef
WORKDIR /app
RUN cargo install cargo-chef --locked

# ------------------------------------------------------------------------------
# Planner: extract dependency recipe from Cargo.toml + Cargo.lock + source
# structure (crate names, binary/example/example targets). Does NOT compile.
# Invalidated on source changes, but runs in seconds.
# ------------------------------------------------------------------------------
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ------------------------------------------------------------------------------
# Builder: compile dependencies from recipe, then the real application source.
# The dependency-cook layer is only invalidated when the dependency tree changes
# (Cargo.toml/Cargo.lock), NOT on routine source edits.
# ------------------------------------------------------------------------------
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo chef cook --release --features cli --bin monocle --recipe-path recipe.json
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release --features cli --bin monocle && \
    cp /app/target/release/monocle /usr/local/bin/monocle

# ------------------------------------------------------------------------------
# Runtime image
# ------------------------------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/bin/monocle /usr/local/bin/monocle

# Run as a non-root user for security
RUN groupadd --system monocle && useradd --system --gid monocle --no-create-home --shell /usr/sbin/nologin monocle
RUN mkdir -p /data/monocle /cache/monocle /home/monocle/.config/monocle \
    && chown -R monocle:monocle /data/monocle /cache/monocle /home/monocle

USER monocle

# Default config: bind to all interfaces, port 8080
ENV MONOCLE_SERVER_ADDRESS=0.0.0.0 \
    MONOCLE_SERVER_PORT=8080

# Data and cache directories (mount as volumes for persistence)
VOLUME ["/data/monocle", "/cache/monocle"]

ENV MONOCLE_DATA_DIR=/data/monocle \
    HOME=/home/monocle

EXPOSE 8080
ENTRYPOINT ["monocle", "server"]
