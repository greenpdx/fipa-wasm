# syntax=docker/dockerfile:1

# FIPA Node Alpine Docker Image
# Multi-stage build for minimal runtime image

# =============================================================================
# Stage 1: Build
# =============================================================================
FROM rust:1.92-alpine AS builder

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    protobuf-dev \
    protoc \
    pkgconfig \
    openssl-dev \
    openssl-libs-static \
    git

# Set protoc path
ENV PROTOC=/usr/bin/protoc

# Create app directory
WORKDIR /build

# Copy source
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY benches/ benches/
COPY build.rs ./
COPY fipa.wit ./
COPY README.md ./

# Build release binary
RUN cargo build --release --bin fipa-node

# Strip binary for smaller size
RUN strip /build/target/release/fipa-node

# =============================================================================
# Stage 2: Runtime
# =============================================================================
FROM alpine:3.21

# Install runtime dependencies
RUN apk add --no-cache \
    ca-certificates \
    libgcc \
    netcat-openbsd

# Create non-root user
RUN addgroup -S fipa && adduser -S fipa -G fipa

# Create directories
RUN mkdir -p /data /config && chown -R fipa:fipa /data /config

# Copy binary from builder
COPY --from=builder /build/target/release/fipa-node /usr/local/bin/fipa-node

# Copy WIT file for reference
COPY --from=builder /build/fipa.wit /usr/share/fipa/fipa.wit

# Switch to non-root user
USER fipa

# Set working directory
WORKDIR /data

# Expose ports
# 9000: gRPC API
# 9090: Prometheus metrics
EXPOSE 9000 9090

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD nc -z localhost 9000 || exit 1

# Default environment
ENV RUST_LOG=info

# Entrypoint
ENTRYPOINT ["fipa-node"]

# Default arguments
CMD ["--listen", "0.0.0.0:9000", "--data-dir", "/data", "--log-format", "json"]
