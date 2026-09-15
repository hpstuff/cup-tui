# Homebrew formula for cup-tui. This repo doubles as the tap:
#   brew tap hpstuff/cup-tui https://github.com/hpstuff/cup-tui
#   brew install cup-tui
# scripts/release.sh updates url/sha256 on every release.
class CupTui < Formula
  desc "Fullscreen terminal UI for ClickUp, built on the cup CLI"
  homepage "https://github.com/hpstuff/cup-tui"
  url "https://github.com/hpstuff/cup-tui/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "d8eb7885ff1ac38baac563082c38bddbc5a8cd0fd665331eb0b84e20ecd22756"
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
