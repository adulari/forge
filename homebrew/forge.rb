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
  version "0.4.2" # release: update sha256 values from checksums.txt after the tag workflow finishes
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-aarch64-apple-darwin.tar.gz"
      sha256 "c78fab82b29aa980e5b6376cff3f40e092f3d0aed12812204a1fa6bf153df2d0"
    end
    on_intel do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-x86_64-apple-darwin.tar.gz"
      sha256 "8869eefcc9761d4aa5ba54171dff077d5c15d3f1cfd759c60f349ada8a417dc1"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "47578ef1b3a73908e8278fe5d7c42d68248a5db99c31e5c819412e79d5a9eaa8"
    end
  end

  def install
    bin.install "forge"
  end

  test do
    assert_match "forge", shell_output("#{bin}/forge --version")
  end
end
