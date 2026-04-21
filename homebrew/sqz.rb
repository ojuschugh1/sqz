# Homebrew formula for sqz — universal context intelligence layer
# Requirement 16.2: brew install sqz
#
# To publish to your own tap:
#   1. Create github.com/ojuschugh1/homebrew-tap
#   2. Add this file as Formula/sqz.rb
#   3. Users install with: brew tap ojuschugh1/tap && brew install sqz
#
# SHA256 checksums must be populated from release artifacts:
#   shasum -a 256 <archive>.tar.gz
class Sqz < Formula
  desc "Universal context intelligence layer — compresses LLM context across CLI, MCP, browser, and IDE"
  homepage "https://github.com/ojuschugh1/sqz"
  version "1.0.1"
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
