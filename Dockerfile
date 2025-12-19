# =============================================================================
# Stage 1: Build
# =============================================================================
FROM rust:1.92-trixie AS builder

WORKDIR /usr/src/monocle

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests first for better layer caching
COPY Cargo.toml Cargo.lock ./

# Create a dummy main and lib to build dependencies
RUN mkdir -p src/bin && \
    echo 'fn main() { println!("dummy"); }' > src/bin/monocle.rs && \
    echo '#![allow(dead_code)]' > src/lib.rs

# Build dependencies only (this layer will be cached)
# Must use same features as final build
RUN cargo build --release --features cli || true
RUN rm -rf src

# Copy the actual source code
COPY src ./src
COPY examples ./examples
COPY README.md ./

# Touch the source files to ensure they're rebuilt
RUN touch src/lib.rs src/bin/monocle.rs

# Build the actual binary
RUN cargo build --release --features cli

# =============================================================================
# Stage 2: Runtime
# =============================================================================
FROM debian:trixie-slim AS runtime

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create a non-root user for security
RUN useradd --create-home --shell /bin/bash monocle

# Create data directory
RUN mkdir -p /data && \
    chown -R monocle:monocle /data

# Copy the binary from builder
COPY --from=builder /usr/src/monocle/target/release/monocle /usr/local/bin/monocle

# Switch to non-root user
USER monocle
WORKDIR /home/monocle

# Set environment variables
ENV MONOCLE_DATA_DIR=/data

# Define volume for persistent data
VOLUME ["/data"]

# Expose the default server port
EXPOSE 8080

# Default command shows help; override with your desired command
ENTRYPOINT ["monocle"]
CMD ["--help"]
