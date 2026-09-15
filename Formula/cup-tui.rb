# Homebrew formula for cup-tui. This repo doubles as the tap:
#   brew tap hpstuff/cup-tui https://github.com/hpstuff/cup-tui
#   brew install cup-tui
# scripts/release.sh updates url/sha256 on every release.
class CupTui < Formula
  desc "Fullscreen terminal UI for ClickUp, built on the cup CLI"
  homepage "https://github.com/hpstuff/cup-tui"
  url "https://github.com/hpstuff/cup-tui/archive/refs/tags/v0.2.0.tar.gz"
  sha256 "e37f6db405558e57b6a63fb9359f186e18493577c4a8c446e575114600d59610"
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
