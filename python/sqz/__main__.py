"""
sqz — universal context intelligence layer
Requirement 16.2: pip install sqz entry point.

Downloads the correct pre-built Rust binary on first run, then delegates to it.
"""

import os
import sys
import platform
import urllib.request
import tarfile
import stat
import subprocess
from pathlib import Path

VERSION = "0.1.0"
REPO = "ojuschugh1/sqz"

# Store the binary alongside this package
_BIN_DIR = Path(__file__).parent / "_bin"


def _platform_target() -> tuple[str, str]:
    """Return (rust_target_triple, binary_extension) for the current platform."""
    system = platform.system()
    machine = platform.machine().lower()

    if system == "Linux":
        if machine in ("x86_64", "amd64"):
            return "x86_64-unknown-linux-musl", ""
        if machine in ("aarch64", "arm64"):
            return "aarch64-unknown-linux-musl", ""
    elif system == "Darwin":
        if machine == "x86_64":
            return "x86_64-apple-darwin", ""
        if machine in ("arm64", "aarch64"):
            return "aarch64-apple-darwin", ""
    elif system == "Windows":
        return "x86_64-pc-windows-msvc", ".exe"

    raise RuntimeError(f"Unsupported platform: {system}/{machine}")


def _download_binary(name: str) -> Path:
    """Download the named binary if not already present; return its path."""
    target, ext = _platform_target()
    binary_path = _BIN_DIR / f"{name}{ext}"

    if binary_path.exists():
        return binary_path

    _BIN_DIR.mkdir(parents=True, exist_ok=True)

    archive_name = f"{name}-v{VERSION}-{target}.tar.gz"
    url = f"https://github.com/{REPO}/releases/download/v{VERSION}/{archive_name}"
    archive_path = _BIN_DIR / archive_name

    print(f"Downloading {name} for {target}...", file=sys.stderr)
    try:
        urllib.request.urlretrieve(url, archive_path)
    except Exception as exc:
        raise RuntimeError(
            f"Failed to download {name} from {url}: {exc}\n"
            "You can manually download from GitHub Releases and place it at "
            f"{binary_path}"
        ) from exc

    with tarfile.open(archive_path, "r:gz") as tar:
        member_name = f"{name}{ext}"
        member = tar.getmember(member_name)
        tar.extract(member, path=_BIN_DIR)

    archive_path.unlink(missing_ok=True)

    # Make executable on Unix
    if ext == "":
        binary_path.chmod(binary_path.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)

    return binary_path


def _run(name: str) -> None:
    """Download (if needed) and exec the named binary, forwarding all args."""
    try:
        binary = _download_binary(name)
    except RuntimeError as exc:
        print(f"Error: {exc}", file=sys.stderr)
        sys.exit(1)

    result = subprocess.run([str(binary)] + sys.argv[1:])
    sys.exit(result.returncode)


def main() -> None:
    """Entry point for the `sqz` command."""
    _run("sqz")


def main_mcp() -> None:
    """Entry point for the `sqz-mcp` command."""
    _run("sqz-mcp")


if __name__ == "__main__":
    main()
