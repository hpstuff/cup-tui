# Homebrew formula for cup-tui. This repo doubles as the tap:
#   brew tap hpstuff/cup-tui https://github.com/hpstuff/cup-tui
#   brew install cup-tui
# scripts/release.sh updates url/sha256 on every release.
class CupTui < Formula
  desc "Fullscreen terminal UI for ClickUp, built on the cup CLI"
  homepage "https://github.com/hpstuff/cup-tui"
  url "https://github.com/hpstuff/cup-tui/archive/refs/tags/v0.2.2.tar.gz"
  sha256 "4a6de84a11185081c23a78bbe0974e27d0f0146c631c2c4fb6f0cbb96b9467ad"
  head "https://github.com/hpstuff/cup-tui.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  def caveats
    <<~EOS
      cup-tui drives the cup CLI, which is installed separately:
        npm install -g @krodak/clickup-cli
        cup init
    EOS
  end

  test do
    assert_match "cup-tui", shell_output("#{bin}/cup-tui --version")
  end
end
