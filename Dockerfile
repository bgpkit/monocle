FROM rust:1-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --features cli --bin monocle

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/monocle /usr/local/bin/monocle
ENV MONOCLE_SERVER_ADDRESS=0.0.0.0 \
    MONOCLE_SERVER_PORT=8080
EXPOSE 8080
ENTRYPOINT ["monocle", "server"]
