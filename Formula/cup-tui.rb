# Homebrew formula for cup-tui. This repo doubles as the tap:
#   brew tap hpstuff/cup-tui https://github.com/hpstuff/cup-tui
#   brew install cup-tui
# scripts/release.sh updates url/sha256 on every release.
class CupTui < Formula
  desc "Fullscreen terminal UI for ClickUp, built on the cup CLI"
  homepage "https://github.com/hpstuff/cup-tui"
  url "https://github.com/hpstuff/cup-tui/archive/refs/tags/v0.2.1.tar.gz"
  sha256 "b299cb325a2aeb1e3a8592719115999c8e9c2247aa8fe8ccd98f975f5857b19c"
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
