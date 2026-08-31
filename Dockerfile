# Multi-stage Dockerfile for Nexus Linux headless MCP server and automated testing

# ------------------------------------------------------------------------------
# 1. Builder Stage: Compile binaries and tests
# ------------------------------------------------------------------------------
FROM rust:1.90-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    libasound2-dev \
    ca-certificates \
    git \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace source files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY xtask ./xtask

# Build nexus-mcp in release mode
RUN cargo build --release --bin nexus-mcp

# ------------------------------------------------------------------------------
# 2. Tester Stage: Run test suite in container
# ------------------------------------------------------------------------------
FROM builder AS tester
CMD ["cargo", "test", "--workspace"]

# ------------------------------------------------------------------------------
# 3. Runner Stage: Lean runtime container for nexus-mcp
# ------------------------------------------------------------------------------
FROM debian:bookworm-slim AS runner

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libasound2 \
    && rm -rf /var/lib/apt/lists/*

# Create standard Linux plugin discovery paths
RUN mkdir -p \
    /root/.clap \
    /root/.vst3 \
    /usr/lib/clap \
    /usr/lib/vst3 \
    /usr/local/lib/clap \
    /usr/local/lib/vst3

# Copy binary from builder
COPY --from=builder /app/target/release/nexus-mcp /usr/local/bin/nexus-mcp

ENV RUST_LOG=info

# Expose Streamable HTTP MCP port
EXPOSE 9997

ENTRYPOINT ["nexus-mcp"]
CMD ["--port", "9997"]
