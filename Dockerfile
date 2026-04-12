# Multi-stage Docker build for sqz.
# Requirement 16.2: Dockerfile distribution channel.
#
# Stage 1 (builder): compiles a statically-linked binary using the musl
#   toolchain so the final image has zero runtime dependencies.
# Stage 2 (runtime): copies only the binary into a minimal scratch image.

# ── Stage 1: builder ──────────────────────────────────────────────────────
FROM rust:1.82-alpine AS builder

# musl-dev provides the musl C library headers needed for static linking.
RUN apk add --no-cache musl-dev

WORKDIR /build

# Cache dependency compilation separately from source compilation.
COPY Cargo.toml Cargo.lock ./
COPY sqz_engine/Cargo.toml sqz_engine/Cargo.toml
COPY sqz/Cargo.toml sqz/Cargo.toml
COPY sqz-mcp/Cargo.toml sqz-mcp/Cargo.toml
COPY sqz-wasm/Cargo.toml sqz-wasm/Cargo.toml

# Create stub source files so `cargo build` can resolve the dependency graph
# without the real source. This layer is cached as long as Cargo.toml files
# do not change.
RUN mkdir -p sqz_engine/src sqz/src sqz-mcp/src sqz-wasm/src \
    && echo 'fn main() {}' > sqz/src/main.rs \
    && echo '' > sqz_engine/src/lib.rs \
    && echo '' > sqz-mcp/src/lib.rs \
    && echo '' > sqz-wasm/src/lib.rs

RUN cargo build --release --target x86_64-unknown-linux-musl --bin sqz 2>/dev/null || true

# Now copy the real source and rebuild only what changed.
COPY . .

# Touch main.rs to force a rebuild of the binary crate.
RUN touch sqz/src/main.rs

RUN cargo build --release --target x86_64-unknown-linux-musl --bin sqz

# ── Stage 2: runtime ──────────────────────────────────────────────────────
FROM scratch

COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/sqz /sqz

ENTRYPOINT ["/sqz"]
