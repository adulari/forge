# Homebrew formula for Forge.
#
#   brew tap florisvoskamp/forge https://github.com/florisvoskamp/forge
#   brew install forge
#
# version + sha256 below are filled in per release (the checksums.txt asset
# produced by .github/workflows/release.yml has the values). Until then, the
# `curl | sh` installer (install.sh) is the recommended path.
class Forge < Formula
  desc "Multi-provider mesh AI coding CLI"
  homepage "https://github.com/florisvoskamp/forge"
  version "0.3.3" # release: update sha256 values from checksums.txt after the tag workflow finishes
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-aarch64-apple-darwin.tar.gz"
      sha256 "81108455c4aa61f644c783cdec6cf3fa990aade3a3f130556e68844ec44ff2b0"
    end
    on_intel do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-x86_64-apple-darwin.tar.gz"
      sha256 "e0350d6191c6cbb77f4278f5aa52bbcf1d030071842dd30f0e89ece6e2c7074f"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "419ebf6b8bc6da14a3221ec962c5e54636508a686f33716af17f8f2ad0aed6fe"
    end
  end

  def install
    bin.install "forge"
  end

  test do
    assert_match "forge", shell_output("#{bin}/forge --version")
  end
end
