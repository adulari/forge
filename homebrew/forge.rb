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
  version "2.5.0" # auto-updated by release.yml (scripts/update-brew-formula.sh) per release
  license "AGPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-aarch64-apple-darwin.tar.gz"
      sha256 "90a43c92934d9ac9d0e1388415432713d5bee8cefaa780a2b4b26b46e87c0ba0"
    end
    on_intel do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-x86_64-apple-darwin.tar.gz"
      sha256 "2328479c7d0e6c0bb74bbcb0c247d82539fd9aef379ba69ab4ef618d656a0dd6"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "283849ec5b8ae4dc00aa74bdbb564413cd91e7381501c5423af0bd2b11024f38"
    end
    on_arm do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "0d65e4a590469dde80fd9852cb0a3588b247f7f92622b59da87ceb772d4d8fe6"
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
