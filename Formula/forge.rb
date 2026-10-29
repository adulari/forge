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
  version "2.25.2"
  license "AGPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-aarch64-apple-darwin.tar.gz"
      sha256 "7d578b96e75d099057938998d24fad81c8e0a51dc570df2385e9e298c708c243"
    end
    on_intel do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-x86_64-apple-darwin.tar.gz"
      sha256 "d67a74bf0787d410d213c9e1d63d346fbd9e1440e35c65293389a50b984c9757"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "d10367b8d92a41b9dba2bedade628758a3a4c4eb7da356e78bbdd08042c5f451"
    end
    on_arm do
      url "https://github.com/Adulari/forge/releases/download/v#{version}/forge-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "f89fca1b15b38b85e0687357cd95e5f66451e6e82bbf907cab81d192d9590d42"
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
