FROM rust:1-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --features cli --bin monocle

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/monocle /usr/local/bin/monocle

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
