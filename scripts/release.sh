#!/usr/bin/env bash
# Cut a release and update the Homebrew formula.
#
#   scripts/release.sh 0.2.0
#
# Steps: bump Cargo.toml, build, commit, tag vX.Y.Z, push, create the GitHub
# release, compute the tarball sha256, update Formula/cup-tui.rb and push.
# This repo is also the Homebrew tap, so that last push is what `brew update`
# picks up. Needs: git push access, `gh` logged in as you, cargo.
set -euo pipefail

VERSION="${1:?usage: scripts/release.sh X.Y.Z}"
REPO="hpstuff/cup-tui"
TAG="v$VERSION"
cd "$(dirname "$0")/.."

[[ -z "$(git status --porcelain)" ]] || { echo "working tree not clean"; exit 1; }
[[ "$(git branch --show-current)" == "main" ]] || { echo "switch to main first"; exit 1; }

echo "▸ bumping Cargo.toml to $VERSION"
sed -i '' -E "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml
cargo build --release
git add Cargo.toml Cargo.lock
if git diff --cached --quiet; then
  echo "  version already $VERSION, nothing to commit"
else
  git commit -m "Release $TAG"
fi
git tag -a "$TAG" -m "cup-tui $TAG"
git push origin main "$TAG"

echo "▸ creating GitHub release"
gh release create "$TAG" --repo "$REPO" --title "cup-tui $TAG" --generate-notes

echo "▸ computing tarball checksum"
URL="https://github.com/$REPO/archive/refs/tags/$TAG.tar.gz"
SHA=$(curl -sL "$URL" | shasum -a 256 | cut -d' ' -f1)
echo "  $SHA"

update_formula() {
  local f="$1"
  sed -i '' -E "s|^  url \".*\"|  url \"$URL\"|" "$f"
  sed -i '' -E "s|^  sha256 \".*\"|  sha256 \"$SHA\"|" "$f"
}

update_formula Formula/cup-tui.rb
git add Formula/cup-tui.rb
git commit -m "Formula: $TAG"
git push origin main

echo "✓ released $TAG"
echo "  new users:      brew tap hpstuff/cup-tui https://github.com/$REPO && brew install cup-tui"
echo "  existing users: brew update && brew upgrade cup-tui"
