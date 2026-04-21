#!/usr/bin/env sh
# sqz — universal context intelligence layer
# Curl-based install script.
# Requirement 16.2: curl-based install script.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/ojuschugh1/sqz/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/ojuschugh1/sqz/main/install.sh | sh -s -- --version 0.1.0
#
# Installs two binaries into $SQZ_INSTALL_DIR (default /usr/local/bin):
#   * sqz     — the CLI (required)
#   * sqz-mcp — the MCP server (optional, warn-and-continue if missing)
#
# sqz-mcp powers MCP-based integrations (Claude Code MCP client, OpenCode,
# etc.). The shell CLI still works if sqz-mcp is unavailable, so a failed
# sqz-mcp download should not abort the install.

set -eu

REPO="ojuschugh1/sqz"
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
            echo "Detected Windows under $OS. install.sh only supports Unix archives (.tar.gz);" >&2
            echo "the Windows release is a .zip. Run the PowerShell installer instead:" >&2
            echo "" >&2
            echo "  irm https://raw.githubusercontent.com/${REPO}/main/install.ps1 | iex" >&2
            echo "" >&2
            echo "Or use npm (works on all platforms, downloads the prebuilt binary):" >&2
            echo "" >&2
            echo "  npm install -g sqz-cli" >&2
            exit 1
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

# ── Download and install a single binary ─────────────────────────────────
# Args: $1 binary name (sqz or sqz-mcp)
#       $2 "required" | "optional"
# Required failures exit the script; optional failures warn and return 1
# so the install can continue with what it has.
install_one_binary() {
    binary="$1"
    required="$2"
    archive="${binary}-${VERSION}-${PLATFORM}.tar.gz"
    url="https://github.com/${REPO}/releases/download/${VERSION}/${archive}"

    tmp_dir="$(mktemp -d)"
    # Local trap so we clean up this binary's tmp dir even if the parent
    # script keeps running for the next binary.
    # shellcheck disable=SC2064
    trap "rm -rf '${tmp_dir}'" EXIT INT TERM

    echo "Downloading ${binary} ${VERSION} for ${PLATFORM}..."
    if ! curl -fsSL "$url" -o "${tmp_dir}/${archive}"; then
        rm -rf "${tmp_dir}"
        trap - EXIT INT TERM
        if [ "$required" = "required" ]; then
            echo "Failed to download required binary ${binary} from ${url}" >&2
            exit 1
        else
            echo "  ! Could not download optional ${binary} (continuing)." >&2
            echo "    MCP-based integrations will be unavailable. To install later:" >&2
            echo "      cargo install ${binary}" >&2
            return 1
        fi
    fi

    echo "Extracting ${binary}..."
    if ! tar -xzf "${tmp_dir}/${archive}" -C "$tmp_dir"; then
        rm -rf "${tmp_dir}"
        trap - EXIT INT TERM
        if [ "$required" = "required" ]; then
            echo "Failed to extract ${archive}" >&2
            exit 1
        else
            echo "  ! Failed to extract optional ${binary} (continuing)." >&2
            return 1
        fi
    fi

    # Locate the binary inside the extracted archive. Two known layouts:
    #
    #   Flat (v1.0.0+):   tar root contains the binary directly.
    #                      ${tmp_dir}/${binary}
    #
    #   Nested (≤v0.9.0): tar root is a directory named after the crate,
    #                      binary is inside it alongside source files.
    #                      ${tmp_dir}/${binary}/${binary}
    #
    # We also handle the case where the top-level entry is a directory
    # (same name as the binary) — move the nested binary up so the rest
    # of the script works unchanged.
    if [ -d "${tmp_dir}/${binary}" ] && [ -f "${tmp_dir}/${binary}/${binary}" ]; then
        # Nested layout: pull the binary up to the expected location.
        mv "${tmp_dir}/${binary}/${binary}" "${tmp_dir}/${binary}.tmp"
        rm -rf "${tmp_dir}/${binary}"
        mv "${tmp_dir}/${binary}.tmp" "${tmp_dir}/${binary}"
    fi

    if [ ! -f "${tmp_dir}/${binary}" ]; then
        # Last resort: search for the binary anywhere in the extracted tree.
        found="$(find "${tmp_dir}" -name "${binary}" -type f | head -n 1)"
        if [ -n "$found" ]; then
            mv "$found" "${tmp_dir}/${binary}"
        else
            rm -rf "${tmp_dir}"
            trap - EXIT INT TERM
            echo "  ! ${archive} did not contain a '${binary}' binary." >&2
            echo "    This is a release-packaging bug — report to https://github.com/${REPO}/issues" >&2
            if [ "$required" = "required" ]; then
                exit 1
            fi
            return 1
        fi
    fi

    echo "Installing to ${INSTALL_DIR}/${binary}..."
    if [ -w "$INSTALL_DIR" ]; then
        mv "${tmp_dir}/${binary}" "${INSTALL_DIR}/${binary}"
        chmod +x "${INSTALL_DIR}/${binary}"
    else
        sudo mv "${tmp_dir}/${binary}" "${INSTALL_DIR}/${binary}"
        sudo chmod +x "${INSTALL_DIR}/${binary}"
    fi

    rm -rf "${tmp_dir}"
    trap - EXIT INT TERM
    echo "  ✓ ${binary} ${VERSION} installed."
    return 0
}

install_everything() {
    # sqz is required — any failure is fatal.
    install_one_binary "sqz" "required"

    # sqz-mcp is optional — soft-fail if the release predates the
    # multi-binary workflow or if the asset is otherwise unavailable.
    install_one_binary "sqz-mcp" "optional" || true

    echo ""
    echo "sqz ${VERSION} installed successfully."
    echo "Next: run 'sqz init' inside a project, or 'sqz init --global' to"
    echo "install hooks for all Claude Code projects."
}

# ── Main ──────────────────────────────────────────────────────────────────

detect_platform
resolve_version
install_everything
