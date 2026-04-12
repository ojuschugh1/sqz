#!/usr/bin/env sh
# sqz — universal context intelligence layer
# Curl-based install script.
# Requirement 16.2: curl-based install script.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/ojuschugh1/sqz/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/ojuschugh1/sqz/main/install.sh | sh -s -- --version 0.1.0

set -eu

REPO="ojuschugh1/sqz"
BINARY="sqz"
INSTALL_DIR="${SQZ_INSTALL_DIR:-/usr/local/bin}"
VERSION="${1:-latest}"

# ── Detect OS and architecture ────────────────────────────────────────────

detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)
            case "$ARCH" in
                x86_64)  PLATFORM="x86_64-unknown-linux-musl" ;;
                aarch64) PLATFORM="aarch64-unknown-linux-musl" ;;
                arm64)   PLATFORM="aarch64-unknown-linux-musl" ;;
                *)
                    echo "Unsupported Linux architecture: $ARCH" >&2
                    exit 1
                    ;;
            esac
            ;;
        Darwin)
            case "$ARCH" in
                x86_64) PLATFORM="x86_64-apple-darwin" ;;
                arm64)  PLATFORM="aarch64-apple-darwin" ;;
                *)
                    echo "Unsupported macOS architecture: $ARCH" >&2
                    exit 1
                    ;;
            esac
            ;;
        MINGW*|MSYS*|CYGWIN*|Windows_NT)
            PLATFORM="x86_64-pc-windows-msvc"
            BINARY="sqz.exe"
            ;;
        *)
            echo "Unsupported operating system: $OS" >&2
            exit 1
            ;;
    esac
}

# ── Resolve latest version ────────────────────────────────────────────────

resolve_version() {
    if [ "$VERSION" = "latest" ]; then
        VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' \
            | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')"
        if [ -z "$VERSION" ]; then
            echo "Could not determine latest release version." >&2
            exit 1
        fi
    fi
}

# ── Download and install ──────────────────────────────────────────────────

download_and_install() {
    ARCHIVE="${BINARY}-${VERSION}-${PLATFORM}.tar.gz"
    URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE}"

    TMP_DIR="$(mktemp -d)"
    trap 'rm -rf "$TMP_DIR"' EXIT

    echo "Downloading sqz ${VERSION} for ${PLATFORM}..."
    curl -fsSL "$URL" -o "${TMP_DIR}/${ARCHIVE}"

    echo "Extracting..."
    tar -xzf "${TMP_DIR}/${ARCHIVE}" -C "$TMP_DIR"

    echo "Installing to ${INSTALL_DIR}/${BINARY}..."
    if [ -w "$INSTALL_DIR" ]; then
        mv "${TMP_DIR}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
        chmod +x "${INSTALL_DIR}/${BINARY}"
    else
        sudo mv "${TMP_DIR}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
        sudo chmod +x "${INSTALL_DIR}/${BINARY}"
    fi

    echo "sqz ${VERSION} installed successfully."
    echo "Run 'sqz init' to configure shell hooks and default presets."
}

# ── Main ──────────────────────────────────────────────────────────────────

detect_platform
resolve_version
download_and_install
