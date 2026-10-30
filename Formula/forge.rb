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
  version "2.26.0"
  license "AGPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-aarch64-apple-darwin.tar.gz"
      sha256 "4ae8c553119d9396e5a0f56ddf9eea3cad22e5aec1aee89f7e76aea1fa75883b"
    end
    on_intel do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-x86_64-apple-darwin.tar.gz"
      sha256 "c1540c8a87a13f92e00c95370fe4f36e03187ed44cf8f60bc5821312d1437ace"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "8a0a18299e459e29d3ae92a83b7e65a73914944cf71403a1c61f29e5a74acf1c"
    end
    on_arm do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "aa5e893eaef3250b3ec5a6855ae2aefef812b4ad023347ff6a7e21cd72468302"
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

  def caveats
    <<~EOS
      Run `forge` to finish setup: the first launch opens a short guided onboarding
      (connect a model, safety and cost, search). `forge setup` reopens it any time.
    EOS
  end

  test do
    assert_match "forge", shell_output("#{bin}/forge --version")
  end
end
