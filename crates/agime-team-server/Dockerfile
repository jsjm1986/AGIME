# Build stage
FROM rust:1.82-slim as builder

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

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 agime

# Create data directory
RUN mkdir -p /data && chown agime:agime /data

# Copy binary
COPY --from=builder /app/target/release/agime-team-server /usr/local/bin/

# Switch to non-root user
USER agime

# Set working directory
WORKDIR /data

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
