# Homebrew formula for sqz — universal context intelligence layer
# Requirement 16.2: brew install sqz
#
# NOTE: SHA256 checksums are placeholders and must be populated from release artifacts before publishing.
# Run: shasum -a 256 <archive>.tar.gz
# Then replace the PLACEHOLDER values below.
class Sqz < Formula
  desc "Universal context intelligence layer — compresses LLM context across CLI, MCP, browser, and IDE"
  homepage "https://github.com/ojuschugh1/sqz"
  version "0.1.0"
  license "ELv2"

  on_macos do
    on_arm do
      url "https://github.com/ojuschugh1/sqz/releases/download/v#{version}/sqz-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_SHA256_AARCH64_DARWIN"
    end
    on_intel do
      url "https://github.com/ojuschugh1/sqz/releases/download/v#{version}/sqz-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_SHA256_X86_64_DARWIN"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/ojuschugh1/sqz/releases/download/v#{version}/sqz-v#{version}-aarch64-unknown-linux-musl.tar.gz"
      sha256 "PLACEHOLDER_SHA256_AARCH64_LINUX"
    end
    on_intel do
      url "https://github.com/ojuschugh1/sqz/releases/download/v#{version}/sqz-v#{version}-x86_64-unknown-linux-musl.tar.gz"
      sha256 "PLACEHOLDER_SHA256_X86_64_LINUX"
    end
  end

  def install
    bin.install "sqz"
    bin.install "sqz-mcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/sqz --version")
  end
end
