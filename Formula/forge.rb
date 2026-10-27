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
  version "2.23.6"
  license "AGPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-aarch64-apple-darwin.tar.gz"
      sha256 "2abfa48710f79c55764fab8284659b97e0a8e283f883d2bd69bef351c1b14c21"
    end
    on_intel do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-x86_64-apple-darwin.tar.gz"
      sha256 "e990822e5732dec6f6fc5bac2cc45b0ac796f957ca09d5a6234831b710d6c145"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "12afebf4213552eb725d92bbdebc7761acfdf0d24b6ab76a75fc496179ab22dd"
    end
    on_arm do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "0b85269ee93db5481f4215e7988504a830f0850f2106991b495de8848af245ae"
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
