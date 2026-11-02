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
  version "2.26.2"
  license "AGPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-aarch64-apple-darwin.tar.gz"
      sha256 "e17837dc7f0a6f18147966bda9fc2f8095b17886294dcfa1128d5f52fdf085a2"
    end
    on_intel do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-x86_64-apple-darwin.tar.gz"
      sha256 "d3eacbbdff3f1a1a6e826b3452f2ea1c210915ae626c424d65ae5eae9b345b72"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "f629a025f94e320c07b173fed0861f51b810b5da8f3146bf15fd469ccaeef366"
    end
    on_arm do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "28629475d6609ba156bd439c30709b4a614878150edcbde4277a3b5bfaea1c9a"
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
