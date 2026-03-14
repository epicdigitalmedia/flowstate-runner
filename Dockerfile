# Stage 1: Build
FROM rust:1.85-bookworm AS builder

WORKDIR /build

# Copy dependency manifests first for layer caching
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && cargo build --release && rm -rf src

# Copy source and rebuild (only recompiles changed code)
COPY src/ src/
RUN cargo build --release

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    jq \
    bash \
    && rm -rf /var/lib/apt/lists/*

# Run as non-root user for security
RUN useradd -m -s /bin/bash runner

COPY --from=builder /build/target/release/flowstate-runner /usr/local/bin/flowstate-runner

# Create project root structure — config and plans are volume-mounted at runtime
RUN mkdir -p /app/.flowstate/plans && chown -R runner:runner /app

WORKDIR /app

ENV HEALTH_PORT=9090
# Default to Kong internal address on Docker network
ENV FLOWSTATE_REST_URL=http://kong:8000
ENV FLOWSTATE_MCP_URL=http://kong:8000/mcp
# API token exchange (set at runtime, not in image)
ENV FLOWSTATE_API_TOKEN=""
ENV FLOWSTATE_AUTH_URL=""

EXPOSE 9090

# Health check — uses HEALTH_PORT env var for configurability
HEALTHCHECK --interval=30s --timeout=3s --retries=3 --start-period=60s \
    CMD curl -f http://localhost:${HEALTH_PORT}/health || exit 1

USER runner

# Default: daemon mode with 60s interval
# --project-root /app expects /app/.flowstate/config.json to be mounted
ENTRYPOINT ["flowstate-runner", "--project-root", "/app"]
CMD ["daemon", "--interval", "60"]
