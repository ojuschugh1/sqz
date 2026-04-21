# sqz — universal context intelligence layer
# PowerShell install script for Windows.
#
# Usage:
#   irm https://raw.githubusercontent.com/ojuschugh1/sqz/main/install.ps1 | iex
#
# Or with an explicit version:
#   $env:SQZ_VERSION = "v0.7.0"; irm https://raw.githubusercontent.com/ojuschugh1/sqz/main/install.ps1 | iex
#
# This installs prebuilt binaries from GitHub Releases. No Rust toolchain,
# no Visual Studio Build Tools, no C compiler required. If you want to
# build from source instead, use `cargo install sqz-cli` / `cargo install
# sqz-mcp` — that path does need the MSVC linker (see README).
#
# Installs two binaries:
#   * sqz.exe     — the CLI (required)
#   * sqz-mcp.exe — the MCP server (optional, warn-and-continue if missing)

$ErrorActionPreference = "Stop"

$Repo    = "ojuschugh1/sqz"
$Version = if ($env:SQZ_VERSION) { $env:SQZ_VERSION } else { "latest" }

# Install to %LOCALAPPDATA%\Programs\sqz\bin by default. This is the user-
# scope convention Microsoft recommends for non-MSI app installs (same
# location winget uses for per-user installs). No admin required.
$InstallDir = if ($env:SQZ_INSTALL_DIR) {
    $env:SQZ_INSTALL_DIR
} else {
    Join-Path $env:LOCALAPPDATA "Programs\sqz\bin"
}

# ── Detect architecture ────────────────────────────────────────────────

# Only x86_64-pc-windows-msvc is published. ARM64 Windows users need to
# either use the emulated x64 binary or build from source.
$Arch = $env:PROCESSOR_ARCHITECTURE
if ($Arch -ne "AMD64") {
    Write-Warning "Only x86_64 Windows binaries are published. Detected: $Arch. Trying x86_64 anyway (may work under emulation)."
}
$Target = "x86_64-pc-windows-msvc"

# ── Resolve latest version ─────────────────────────────────────────────

if ($Version -eq "latest") {
    Write-Host "Resolving latest release..."
    try {
        $LatestUrl = "https://api.github.com/repos/$Repo/releases/latest"
        $Response  = Invoke-RestMethod -Uri $LatestUrl -Headers @{ "User-Agent" = "sqz-installer" }
        $Version   = $Response.tag_name
    } catch {
        Write-Error "Could not determine latest release: $_"
        exit 1
    }
    if (-not $Version) {
        Write-Error "Latest release has no tag_name — GitHub API may be rate-limited. Retry later or set SQZ_VERSION explicitly."
        exit 1
    }
}
Write-Host "Installing sqz $Version for $Target"

# Ensure install dir exists once up front.
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

# ── Install a single binary ────────────────────────────────────────────
#
# Returns $true on success, $false on non-fatal failure. Throws for
# required binaries (via Write-Error) so the script exits non-zero.

function Install-SqzBinary {
    param(
        [Parameter(Mandatory=$true)][string]$Name,
        [Parameter(Mandatory=$true)][string]$Version,
        [Parameter(Mandatory=$true)][string]$Target,
        [Parameter(Mandatory=$true)][string]$InstallDir,
        [Parameter(Mandatory=$true)][bool]$Required
    )

    $BinaryFile = "$Name.exe"
    $Archive    = "$Name-$Version-$Target.zip"
    $Url        = "https://github.com/$script:Repo/releases/download/$Version/$Archive"

    $TmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ("sqz-install-" + [System.Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null
    try {
        $ZipPath = Join-Path $TmpDir $Archive
        Write-Host "Downloading $Url"
        try {
            Invoke-WebRequest -Uri $Url -OutFile $ZipPath -UseBasicParsing
        } catch {
            if ($Required) {
                Write-Error "Failed to download required binary ${Name}: $_"
                exit 1
            }
            Write-Warning "Could not download optional $Name (continuing)."
            Write-Warning "  MCP-based integrations will be unavailable."
            Write-Warning "  To install later, run: cargo install $Name"
            return $false
        }

        Write-Host "Extracting $Name..."
        Expand-Archive -Path $ZipPath -DestinationPath $TmpDir -Force

        $SrcBinary = Join-Path $TmpDir $BinaryFile
        if (-not (Test-Path $SrcBinary -PathType Leaf)) {
            # Sanity check: must be a file. If the release tarball layout
            # ever changes to contain a nested directory, catch it here
            # rather than letting Copy-Item silently misbehave.
            Write-Warning "$Archive did not contain a top-level '$BinaryFile' file."
            Write-Warning "This is a release-packaging bug — report to https://github.com/$script:Repo/issues"
            if ($Required) {
                Write-Error "Required binary $Name is missing from the archive."
                exit 1
            }
            return $false
        }

        $DestBinary = Join-Path $InstallDir $BinaryFile
        try {
            Copy-Item -Path $SrcBinary -Destination $DestBinary -Force
        } catch [System.IO.IOException] {
            # Common on Windows: the existing binary is in use by a
            # running shell hook, the dashboard, or the API proxy.
            Write-Error @"
Could not overwrite $DestBinary — the file is likely in use.

If $Name is currently running (a shell hook, the dashboard, or an MCP
client such as Claude Code), close those processes and re-run the
installer. On Windows, running binaries cannot be replaced while open.
"@
            if ($Required) { exit 1 }
            return $false
        }
        Write-Host "  installed: $DestBinary"
        return $true
    }
    finally {
        Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue
    }
}

# ── Install binaries ───────────────────────────────────────────────────

# sqz is required.
$null = Install-SqzBinary -Name "sqz" -Version $Version -Target $Target `
    -InstallDir $InstallDir -Required $true

# sqz-mcp is optional — soft-fail for releases before v0.10.0 that did
# not ship this tarball.
$null = Install-SqzBinary -Name "sqz-mcp" -Version $Version -Target $Target `
    -InstallDir $InstallDir -Required $false

# ── Add install dir to user PATH if not already there ──────────────────

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (-not $UserPath) { $UserPath = "" }

$PathEntries = $UserPath.Split(";") | Where-Object { $_ -ne "" }
$AlreadyOnPath = $PathEntries | Where-Object { $_ -ieq $InstallDir }

if (-not $AlreadyOnPath) {
    Write-Host "Adding $InstallDir to user PATH"
    $NewPath = if ($UserPath) { "$UserPath;$InstallDir" } else { $InstallDir }
    [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
    Write-Host ""
    Write-Host "PATH updated. Restart your shell (or log out and back in) for the change to take effect."
} else {
    Write-Host "$InstallDir is already on PATH."
}

Write-Host ""
Write-Host "sqz $Version installed successfully."
Write-Host "Next: run 'sqz init' inside a project, or 'sqz init --global' to"
Write-Host "install hooks for all Claude Code projects."
