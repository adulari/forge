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
  version "2.24.1"
  license "AGPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-aarch64-apple-darwin.tar.gz"
      sha256 "e1b58a81dcd5ae798b7b156a9eeac8fbaa33c145436843fdae77f068be6c724e"
    end
    on_intel do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-x86_64-apple-darwin.tar.gz"
      sha256 "1aa40b92987025c52f7d2f8a63932dc9ff6cde7be32f1815b1e71c872dbcd6f5"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "cad4a33abe8f1015c7bcd573e1b28c8c43bc700a4e3b621353ddc86ba60281f2"
    end
    on_arm do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "f4ae925aeac7966e1d4c109b9871c8cd429bc5b00c040b38021bfc37aa20eeb1"
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
