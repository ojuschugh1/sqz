# Container image for the sqz MCP server (sqz-mcp, stdio transport).
#
# Used by the Docker MCP catalog (docker/mcp-registry) and publishable to
# ghcr.io for the official MCP Registry's OCI package entry. The
# io.modelcontextprotocol.server.name label is how the MCP Registry
# verifies image ownership; it must match the server name in
# distribution/mcp-registry/server.json.
#
# Build (from repo root):
#   docker build -f distribution/docker/Dockerfile -t ghcr.io/ojuschugh1/sqz-mcp:latest .
# Run:
#   docker run -i --rm ghcr.io/ojuschugh1/sqz-mcp

# ── Stage 1: builder ──────────────────────────────────────────────────────
FROM rust:1.90-alpine AS builder

RUN apk add --no-cache musl-dev

WORKDIR /build
COPY . .

RUN cargo build --release --bin sqz-mcp

# ── Stage 2: runtime ──────────────────────────────────────────────────────
# Alpine rather than scratch: sqz-mcp persists its dedup cache under
# $HOME/.sqz, so the image needs a writable home directory.
FROM alpine:3.20

LABEL io.modelcontextprotocol.server.name="io.github.ojuschugh1/sqz"
LABEL org.opencontainers.image.source="https://github.com/ojuschugh1/sqz"
LABEL org.opencontainers.image.description="sqz MCP server — pre-injection LLM context compression"
LABEL org.opencontainers.image.licenses="Elastic-2.0"

RUN adduser -D sqz
USER sqz
ENV HOME=/home/sqz

COPY --from=builder /build/target/release/sqz-mcp /usr/local/bin/sqz-mcp

ENTRYPOINT ["/usr/local/bin/sqz-mcp"]
