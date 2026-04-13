#!/usr/bin/env node
// postinstall script — downloads the correct sqz binary for the current platform
// Requirement 16.2: npm install -g sqz-cli / npx sqz-cli wrapper

"use strict";

const https = require("https");
const fs = require("fs");
const path = require("path");
const { execSync } = require("child_process");

const REPO = "ojuschugh1/sqz";
const VERSION = require("./package.json").version;
const BIN_DIR = path.join(__dirname, "bin");

// Map Node.js platform/arch to Rust target triples
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

async function install() {
  const { target, ext } = getPlatformTarget();

  // Ensure bin directory exists
  if (!fs.existsSync(BIN_DIR)) {
    fs.mkdirSync(BIN_DIR, { recursive: true });
  }

  const baseUrl = `https://github.com/${REPO}/releases/download/v${VERSION}`;

  for (const name of ["sqz", "sqz-mcp"]) {
    const archive = `${name}-v${VERSION}-${target}.tar.gz`;
    const url = `${baseUrl}/${archive}`;
    const archivePath = path.join(BIN_DIR, archive);
    const binaryName = `${name}${ext}`;
    const binaryDest = path.join(BIN_DIR, binaryName);

    console.log(`Downloading ${name} for ${target}...`);
    try {
      await downloadFile(url, archivePath);

      // Extract the binary from the tarball
      execSync(`tar -xzf "${archivePath}" -C "${BIN_DIR}" "${binaryName}"`, { stdio: "inherit" });
      fs.unlinkSync(archivePath);

      if (process.platform !== "win32") {
        fs.chmodSync(binaryDest, 0o755);
      }
      console.log(`  ✓ ${name} installed`);
    } catch (err) {
      console.error(`  ✗ Failed to download ${name}: ${err.message}`);
      console.error(`    You can manually download from: ${url}`);
      process.exit(1);
    }
  }
}

install().catch((err) => {
  console.error("sqz install failed:", err.message);
  process.exit(1);
});
