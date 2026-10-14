# Homebrew tap formula for Forge.
#
#   brew tap Adulari/forge https://github.com/Adulari/forge
#   brew install Adulari/forge/forge
#
# Version and SHA-256 values are updated transactionally from each GitHub release's
# checksums.txt by scripts/update-package-manifests.sh.
class Forge < Formula
  desc "Multi-provider mesh AI coding CLI"
  homepage "https://github.com/Adulari/forge"
  version "2.19.0"
  license "AGPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-aarch64-apple-darwin.tar.gz"
      sha256 "a40609d240da8ab07b34f5710b2215f1e417ea5d05320e365e4ba01b5600f692"
    end
    on_intel do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-x86_64-apple-darwin.tar.gz"
      sha256 "a6a18af46697137b912a27263d5f6c39608c76f2844b4b91bb233116e2141547"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "6606641cc3345395d111ae439126f89c0f8d99ff5ffde3b2360321bf0a641371"
    end
    on_arm do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "4b930f3e507daea0ac2cd6a2d6aa46f537635a41a1c89aa5284fc2e5b14f519a"
    end
  end

  def install
    bin.install "forge"
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
