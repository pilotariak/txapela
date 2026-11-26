# SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
# SPDX-License-Identifier: Apache-2.0

# Multi-stage build for Rust applications
# Stage 1: Build
FROM rust:1.97-alpine AS builder

# Install build dependencies
RUN apk add --no-cache musl-dev

# Create a new directory for the application
WORKDIR /usr/src/app

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Create a dummy main.rs to cache dependencies
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Copy source code
COPY src ./src

# Build the application
# Touch main.rs to force rebuild with actual source
RUN touch src/main.rs && \
    cargo build --release

# Stage 2: Runtime
FROM alpine:3.23

# Install runtime dependencies if needed
# RUN apk add --no-cache ca-certificates

# Create non-root user
RUN addgroup -g 1000 app && \
    adduser -D -u 1000 -G app app

# Set working directory
WORKDIR /app

# Copy binary from builder
COPY --from=builder /usr/src/app/target/release/txapela /app/txapela

# Change ownership
RUN chown -R app:app /app

# Switch to non-root user
USER app

# Expose port if your application serves HTTP
# EXPOSE 8080

# Health check (adjust according to your application)
# HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
#   CMD wget --no-verbose --tries=1 --spider http://localhost:8080/health || exit 1

# Run the binary
ENTRYPOINT ["/app/txapela"]
