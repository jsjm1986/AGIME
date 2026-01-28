# Stage 1: Build web-admin frontend
FROM node:20-slim as frontend-builder

WORKDIR /app/web-admin

# Copy web-admin source
COPY crates/agime-team-server/web-admin/package*.json ./
RUN npm ci

COPY crates/agime-team-server/web-admin ./
RUN npm run build

# Stage 2: Build Rust backend
FROM rust:1.85-slim as builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build the team server
RUN cargo build --release -p agime-team-server

# Stage 3: Runtime
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 agime

# Create data directory
RUN mkdir -p /data && chown agime:agime /data

# Create web-admin directory
RUN mkdir -p /app/web-admin/dist && chown -R agime:agime /app

# Copy binary
COPY --from=builder /app/target/release/agime-team-server /usr/local/bin/

# Copy web-admin dist
COPY --from=frontend-builder /app/web-admin/dist /app/web-admin/dist

# Switch to non-root user
USER agime

# Set working directory (server looks for ./web-admin/dist)
WORKDIR /app

# Expose port
EXPOSE 8080

# Environment variables
ENV TEAM_SERVER_HOST=0.0.0.0
ENV TEAM_SERVER_PORT=8080
ENV DATABASE_URL=sqlite:///data/team.db?mode=rwc

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# Run the server
CMD ["agime-team-server"]
