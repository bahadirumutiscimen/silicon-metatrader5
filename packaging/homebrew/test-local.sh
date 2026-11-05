#!/bin/bash
# Try the Homebrew formula with this checkout, before a release exists:
# packs the working tree as a source tarball, renders siliconmt5.rb into a
# local tap (smt5/local) and installs it from source.
#
#   bash packaging/homebrew/test-local.sh
#   # undo: brew uninstall siliconmt5 && brew untap smt5/local
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/app/Cargo.toml" | head -1)"
NAME="silicon-metatrader5-$VERSION"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK/$NAME"' EXIT

echo "==> source tarball ($VERSION) from $ROOT"
mkdir -p "$WORK/$NAME"
# only what git would ship: tracked and new files, minus everything ignored
git -C "$ROOT" ls-files -z --cached --others --exclude-standard \
  | rsync -a --from0 --ignore-missing-args --files-from=- "$ROOT/" "$WORK/$NAME/"
tar -C "$WORK" -czf "$WORK/$NAME.tar.gz" "$NAME"
SHA="$(shasum -a 256 "$WORK/$NAME.tar.gz" | awk '{print $1}')"

echo "==> local tap smt5/local"
brew tap-new --no-git smt5/local >/dev/null 2>&1 || true
TAP="$(brew --repository)/Library/Taps/smt5/homebrew-local/Formula"
mkdir -p "$TAP"
sed -e "s|https://github.com/bahadirumutiscimen/silicon-metatrader5/archive/refs/tags/v@VERSION@.tar.gz|file://$WORK/$NAME.tar.gz|" \
    -e "s|@SHA256@|$SHA|" -e "s|@VERSION@|$VERSION|" \
    "$ROOT/packaging/homebrew/siliconmt5.rb" >"$TAP/siliconmt5.rb"

echo "==> brew install (builds from source, a few minutes)"
if brew list --formula siliconmt5 >/dev/null 2>&1; then
  brew reinstall --build-from-source smt5/local/siliconmt5
else
  brew install --build-from-source smt5/local/siliconmt5
fi
brew test smt5/local/siliconmt5
echo "OK. App: $(brew --prefix)/opt/siliconmt5/SiliconMetaTrader5.app"
