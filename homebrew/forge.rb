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
  version "2.5.7" # auto-updated by release.yml (scripts/update-brew-formula.sh) per release
  license "AGPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-aarch64-apple-darwin.tar.gz"
      sha256 "822dff37d6bac62b8ed59fee5f643f1add227b6ab49a4c67c203a72da6cc1867"
    end
    on_intel do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-x86_64-apple-darwin.tar.gz"
      sha256 "16311cab68f7a64555024a9fe0285ca441e2152a5abf67468dbe5cfa91c90f8d"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "52f311e1ab5c043e21db92b3d82e0fb788604b07bb2a1f840f6f2401a768177d"
    end
    on_arm do
      url "https://github.com/florisvoskamp/forge/releases/download/v#{version}/forge-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "94d27ee27422043cf2a93e504e55f2227c3d69f9e354534dd0f314301b9f1a27"
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
