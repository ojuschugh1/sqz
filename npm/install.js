#!/usr/bin/env node
// postinstall script — downloads the correct sqz binaries for the current
// platform. Ships two binaries:
//
//   * sqz     — the CLI (required, install fails if this is missing)
//   * sqz-mcp — the MCP server (optional, we log and continue if missing)
//
// The distinction matters because sqz (the CLI) is the core value — it
// compresses shell output via `sqz compress` and powers the hook system.
// sqz-mcp is a separate feature (MCP protocol for tools like Claude Code,
// Cursor, OpenCode). A user who only cares about shell compression
// should not have their install fail if the MCP server tarball is
// unavailable for any reason (e.g. installing an older release tagged
// before sqz-mcp was packaged — see issue shochdoerfer/76vangel, fixed
// in 0.9.x release workflow).
//
// Each archive contains one binary at the root, no wrapping directory,
// so the install extracts `sqz` (or `sqz.exe`) directly into bin/.
// This matches the layout the GitHub release workflow produces —
// keep `.github/workflows/release.yml` and this file in sync.

"use strict";

const https = require("https");
const fs = require("fs");
const path = require("path");
const { execSync } = require("child_process");

const REPO = "ojuschugh1/sqz";
const VERSION = require("./package.json").version;
const BIN_DIR = path.join(__dirname, "bin");

// Map Node.js platform/arch to Rust target triples (same naming as the
// release workflow).
function getPlatformTarget() {
  const platform = process.platform;
  const arch = process.arch;

  if (platform === "linux") {
    if (arch === "x64") return { target: "x86_64-unknown-linux-musl", ext: "" };
    if (arch === "arm64") return { target: "aarch64-unknown-linux-musl", ext: "" };
  }
  if (platform === "darwin") {
    if (arch === "x64") return { target: "x86_64-apple-darwin", ext: "" };
    if (arch === "arm64") return { target: "aarch64-apple-darwin", ext: "" };
  }
  if (platform === "win32") {
    return { target: "x86_64-pc-windows-msvc", ext: ".exe" };
  }

  throw new Error(`Unsupported platform: ${platform}/${arch}`);
}

function downloadFile(url, dest) {
  return new Promise((resolve, reject) => {
    const file = fs.createWriteStream(dest);
    const request = (u) => {
      https.get(u, (res) => {
        // Follow redirects (GitHub Releases uses them)
        if (res.statusCode === 301 || res.statusCode === 302) {
          file.close();
          return request(res.headers.location);
        }
        if (res.statusCode !== 200) {
          file.close();
          fs.unlink(dest, () => {});
          return reject(new Error(`HTTP ${res.statusCode} downloading ${u}`));
        }
        res.pipe(file);
        file.on("finish", () => file.close(resolve));
      }).on("error", (err) => {
        file.close();
        fs.unlink(dest, () => {});
        reject(err);
      });
    };
    request(url);
  });
}

// Extract a single named file from a tarball into BIN_DIR. On Windows,
// zip archives are used instead — `tar` on recent Windows 10+ can read
// both .tar.gz and .zip, so we use the same command everywhere.
//
// Handles two known archive layouts:
//   Flat (v1.0.0+):   binary at archive root → tar contains "sqz"
//   Nested (≤v0.9.0): binary inside a subdirectory → tar contains "sqz/sqz"
function extractBinary(archivePath, binaryName) {
  const archive = path.basename(archivePath);
  if (archive.endsWith(".zip") && process.platform === "win32") {
    // PowerShell's Expand-Archive is the reliable path on Windows —
    // tar.exe can choke on some zip metadata produced by Compress-Archive.
    execSync(
      `powershell -NoProfile -NonInteractive -Command ` +
        `"Expand-Archive -Path '${archivePath}' ` +
        `-DestinationPath '${BIN_DIR}' -Force"`,
      { stdio: "inherit" }
    );
    // After extraction, the binary might be nested in a subdirectory.
    const binaryDest = path.join(BIN_DIR, binaryName);
    if (!fs.existsSync(binaryDest)) {
      // Look for it inside a subdirectory (e.g. sqz/sqz.exe)
      const baseName = binaryName.replace(/\.exe$/, "");
      const nestedPath = path.join(BIN_DIR, baseName, binaryName);
      if (fs.existsSync(nestedPath)) {
        fs.renameSync(nestedPath, binaryDest);
        // Clean up the now-empty subdirectory.
        try { fs.rmdirSync(path.join(BIN_DIR, baseName), { recursive: true }); }
        catch (_) { /* ignore */ }
      }
    }
    return;
  }
  // Try extracting the binary from the top level first.
  try {
    execSync(`tar -xzf "${archivePath}" -C "${BIN_DIR}" "${binaryName}"`, {
      stdio: "inherit",
    });
    return;
  } catch (_) {
    // Flat extraction failed — try the nested layout.
  }

  // Nested layout: extract everything, then move the binary up.
  const tmpExtract = path.join(BIN_DIR, "__sqz_extract_tmp__");
  if (!fs.existsSync(tmpExtract)) {
    fs.mkdirSync(tmpExtract, { recursive: true });
  }
  execSync(`tar -xzf "${archivePath}" -C "${tmpExtract}"`, {
    stdio: "inherit",
  });

  // Search for the binary inside the extracted tree.
  const binaryDest = path.join(BIN_DIR, binaryName);
  const baseName = binaryName.replace(/\.exe$/, "");
  const candidates = [
    path.join(tmpExtract, baseName, binaryName),  // sqz/sqz
    path.join(tmpExtract, binaryName),             // sqz (shouldn't happen if flat failed, but be safe)
  ];

  let found = false;
  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) {
      fs.renameSync(candidate, binaryDest);
      found = true;
      break;
    }
  }

  // Clean up temp extraction directory.
  try { fs.rmSync(tmpExtract, { recursive: true, force: true }); }
  catch (_) { /* ignore */ }

  if (!found) {
    throw new Error(
      `${archive} did not contain '${binaryName}' at the expected location. ` +
      `This is a release-packaging bug — report to https://github.com/${REPO}/issues`
    );
  }
}

// Download and install a single binary.
// Returns true on success, false on failure. Throws only for `required`
// binaries; optional ones log and return false so the rest of the install
// continues.
async function installBinary(name, target, ext, baseUrl, { required }) {
  const archiveExt = process.platform === "win32" ? "zip" : "tar.gz";
  const archive = `${name}-v${VERSION}-${target}.${archiveExt}`;
  const url = `${baseUrl}/${archive}`;
  const archivePath = path.join(BIN_DIR, archive);
  const binaryName = `${name}${ext}`;
  const binaryDest = path.join(BIN_DIR, binaryName);

  // Remove any stale placeholder wrapper file that was shipped inside
  // the npm tarball. We will overwrite it with the real binary;
  // failing to remove first can cause "cannot overwrite" errors on
  // platforms where the wrapper was chmod-ed executable.
  try {
    if (fs.existsSync(binaryDest)) fs.unlinkSync(binaryDest);
  } catch (_) {
    // Non-fatal — extractBinary will surface any real error.
  }

  console.log(`Downloading ${name} for ${target}...`);
  try {
    await downloadFile(url, archivePath);
    extractBinary(archivePath, binaryName);
    fs.unlinkSync(archivePath);

    if (process.platform !== "win32") {
      fs.chmodSync(binaryDest, 0o755);
    }
    console.log(`  ✓ ${name} installed`);
    return true;
  } catch (err) {
    // Clean up any partial download so a retry starts fresh.
    try { if (fs.existsSync(archivePath)) fs.unlinkSync(archivePath); }
    catch (_) { /* ignore */ }

    if (required) {
      console.error(`  ✗ Failed to download ${name}: ${err.message}`);
      console.error(`    You can manually download from: ${url}`);
      throw err;
    } else {
      // Optional binary — warn loudly but don't break the install.
      // This is the case for `sqz-mcp` on releases that predate the
      // multi-binary workflow (anything before v0.10.0).
      console.warn(`  ! ${name} could not be downloaded (optional): ${err.message}`);
      console.warn(`    MCP-based integrations (Claude Code MCP, OpenCode, etc.)`);
      console.warn(`    will be unavailable. The sqz CLI itself still works.`);
      console.warn(`    If you need ${name}, install manually:`);
      console.warn(`      cargo install sqz-mcp`);
      console.warn(`    or grab the binary from:`);
      console.warn(`      ${url}`);
      return false;
    }
  }
}

async function install() {
  const { target, ext } = getPlatformTarget();

  // Ensure bin directory exists
  if (!fs.existsSync(BIN_DIR)) {
    fs.mkdirSync(BIN_DIR, { recursive: true });
  }

  const baseUrl = `https://github.com/${REPO}/releases/download/v${VERSION}`;

  // sqz is required. If this download fails the package is useless —
  // propagate the error so `npm install` exits non-zero.
  await installBinary("sqz", target, ext, baseUrl, { required: true });

  // sqz-mcp is optional. Releases before v0.10.0 do not ship this
  // tarball and there is no point failing the whole install over it.
  await installBinary("sqz-mcp", target, ext, baseUrl, { required: false });
}

install().catch((err) => {
  console.error("sqz install failed:", err.message);
  process.exit(1);
});
