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
  version "2.6.5" # auto-updated by release.yml (scripts/update-brew-formula.sh) per release
  license "AGPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-aarch64-apple-darwin.tar.gz"
      sha256 "ec377ea0d413407e0bf311f00459f493c731aa16e76b92c281396f2f77c3c9a6"
    end
    on_intel do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-x86_64-apple-darwin.tar.gz"
      sha256 "d10a7554207b41bdb0b57de0227d593c3499368fa80a843b3968aa0cb2f4240f"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "76007d37b0a87dd7fb01a0ed7ab8f480f77acde3882df1fa980c5522d817242c"
    end
    on_arm do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "f9c27ecf9b6f3b264f503bd117b69283f0dc96cb3ec09a8e4fe317aa3009f81c"
    end
  end

  def install
    bin.install "forge"
    # Completions + man page are bundled in releases that built them (xtasks gen-dist, wired into
    # release.yml). Guard so the formula still installs from older asset sets without them.
    if File.exist?("completions/forge.bash")
      bash_completion.install "completions/forge.bash" => "forge"
      zsh_completion.install "completions/_forge"
      fish_completion.install "completions/forge.fish"
    end
    man1.install "forge.1" if File.exist?("forge.1")
  end

  test do
    assert_match "forge", shell_output("#{bin}/forge --version")
  end
end
