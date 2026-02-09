# Titan Node - Multi-stage Docker build
# Produces a minimal production image

# ============ Builder Stage ============
FROM rust:1.75-bookworm AS builder

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .cargo ./.cargo
COPY crates ./crates

# Build release binary
RUN cargo build --release --bin titan-node

# ============ Runtime Stage ============  
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create data directories
RUN mkdir -p /data/snapshots

# Copy binary from builder
COPY --from=builder /app/target/release/titan-node /usr/local/bin/titan-node

# Set working directory
WORKDIR /app

# Expose ports
# 8080: TCP Gateway (order ingestion)
# 9090: Metrics (Prometheus scrape target)
EXPOSE 8080 9090

# Health check
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:9090/health || exit 1

# Run as non-root user
RUN useradd -r -s /bin/false titan
USER titan

# Environment
ENV RUST_LOG=info

# Entrypoint
CMD ["titan-node"]
