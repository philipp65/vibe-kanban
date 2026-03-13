# syntax=docker/dockerfile:1.6

# Build stage
FROM node:24-alpine AS builder

# Install build dependencies
RUN apk add --no-cache \
    curl \
    build-base \
    perl \
    llvm-dev \
    clang-dev \
    rust \
    cargo

# Allow linking libclang on musl
ENV RUSTFLAGS="-C target-feature=-crt-static"

ARG POSTHOG_API_KEY
ARG POSTHOG_API_ENDPOINT

ENV VITE_PUBLIC_POSTHOG_KEY=$POSTHOG_API_KEY
ENV VITE_PUBLIC_POSTHOG_HOST=$POSTHOG_API_ENDPOINT

# Set working directory
WORKDIR /app

# Copy package files for dependency caching
COPY package*.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY packages/local-web/package*.json ./packages/local-web/
COPY packages/web-core/package*.json ./packages/web-core/
COPY packages/ui/package*.json ./packages/ui/
COPY npx-cli/package*.json ./npx-cli/

# Install pnpm and dependencies
RUN --mount=type=cache,target=/root/.npm \
    --mount=type=cache,target=/pnpm/store \
    npm install -g pnpm && \
    pnpm config set store-dir /pnpm/store && \
    pnpm install --frozen-lockfile

# Copy only sources needed for local-web + server build to
# avoid invalidating cache from unrelated repository changes.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ ./crates/
COPY shared/ ./shared/
COPY packages/local-web/ ./packages/local-web/
COPY packages/web-core/ ./packages/web-core/
COPY packages/ui/ ./packages/ui/
COPY packages/public/ ./packages/public/
COPY scripts/ ./scripts/
COPY assets/ ./assets/
COPY npx-cli/ ./npx-cli/

# Build application
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    npm run generate-types
RUN cd packages/local-web && pnpm run build
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release --bin server && \
    mkdir -p /app/bin && \
    cp /app/target/release/server /app/bin/server

# Runtime stage
FROM alpine:latest AS runtime

# Install runtime dependencies
RUN apk add --no-cache \
    ca-certificates \
    tini \
    su-exec \
    libgcc \
    wget \
    git \
    glab

# Create app user for security
RUN addgroup -g 1001 -S appgroup && \
    adduser -u 1001 -S appuser -G appgroup

# Copy binary from builder (persisted outside target cache mount)
COPY --from=builder /app/bin/server /usr/local/bin/server

# Prepare writable runtime directories for appuser
ENV HOME=/home/appuser
ENV XDG_DATA_HOME=/home/appuser/.local/share
ENV XDG_CACHE_HOME=/tmp/vibe-kanban-cache
ENV VIBEKANBAN_ASSET_DIR=/repos/.vibe-kanban-assets
RUN mkdir -p /repos /repos/.vibe-kanban-assets /home/appuser/.local/share /tmp/vibe-kanban-cache && \
    chown -R appuser:appgroup /repos /home/appuser /tmp/vibe-kanban-cache

# Ensure bind-mounted host paths are writable on every boot.
RUN cat <<'EOF' > /usr/local/bin/docker-entrypoint.sh
#!/bin/sh
set -eu

mkdir -p /repos /repos/.vibe-kanban-assets /home/appuser/.local/share /tmp/vibe-kanban-cache
chown -R appuser:appgroup /repos /home/appuser /tmp/vibe-kanban-cache

exec su-exec appuser "$@"
EOF
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

# Set runtime environment
ENV HOST=0.0.0.0
ENV PORT=3000
EXPOSE 3000

# Set working directory
WORKDIR /repos

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget --quiet --tries=1 --spider "http://${HOST:-localhost}:${PORT:-3000}" || exit 1

# Run the application
ENTRYPOINT ["/sbin/tini", "--", "/usr/local/bin/docker-entrypoint.sh"]
CMD ["server"]
