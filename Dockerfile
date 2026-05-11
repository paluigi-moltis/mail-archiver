# Stage 1: Build
FROM rust:1.83-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy manifests first for layer caching
COPY Cargo.toml Cargo.lock ./

# Create a dummy main.rs to build dependencies
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs
RUN cargo build --release && rm -rf src

# Copy actual source
COPY . .

# Touch source files to invalidate the cache for non-dep files
RUN touch src/main.rs

# Build the real binary
RUN cargo build --release

# Stage 2: Runtime
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/mail-archive /app/mail-archive

# Copy migrations and templates
COPY migrations /app/migrations
COPY templates /app/templates
COPY static /app/static

# Create data directory
RUN mkdir -p /data/attachments

# Environment
ENV DATA_DIR=/data
ENV HOST=0.0.0.0
ENV PORT=8000

EXPOSE 8000

VOLUME ["/data"]

CMD ["/app/mail-archive"]
