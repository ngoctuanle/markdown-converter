#!/usr/bin/env bash
# Build a distributable package with the bundled pandoc.
# Usage: scripts/package.sh <target-triple>
#   aarch64-apple-darwin | x86_64-apple-darwin | universal-apple-darwin (macOS: app,dmg)
#   x86_64-pc-windows-msvc (Windows: nsis) | x86_64-unknown-linux-gnu (Linux: deb,appimage)
# Each OS builds only its own formats. Requires: cargo install cargo-packager --locked --version 0.11.8
# On Windows, prefer scripts\package.ps1: no bash needed, and it also makes a portable zip.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
triple="${1:?usage: scripts/package.sh <target-triple>}"

refresh_current() { # $1 = vendor dir name to copy into vendor/pandoc/current
  rm -rf vendor/pandoc/current
  mkdir -p vendor/pandoc/current
  cp -R "vendor/pandoc/$1/." vendor/pandoc/current/
  # Cargo.toml's packager resources expect the licence under its shipped name.
  mv vendor/pandoc/current/COPYING.md vendor/pandoc/current/pandoc-COPYING.md
}

if [[ "$triple" == "universal-apple-darwin" ]]; then
  arm=aarch64-apple-darwin
  x86=x86_64-apple-darwin
  scripts/fetch-pandoc.sh "$arm"
  scripts/fetch-pandoc.sh "$x86"
  mkdir -p vendor/pandoc/universal-apple-darwin
  lipo -create "vendor/pandoc/$arm/pandoc" "vendor/pandoc/$x86/pandoc" \
    -output vendor/pandoc/universal-apple-darwin/pandoc
  cp "vendor/pandoc/$arm/COPYING.md" vendor/pandoc/universal-apple-darwin/COPYING.md
  refresh_current universal-apple-darwin

  rustup target add "$arm" "$x86"
  cargo build --release --target "$arm"
  cargo build --release --target "$x86"
  # Package the .app from the arm64 build, then swap in the universal binary.
  cargo packager --release --target "$arm" --formats app
  app="dist/Markdown Converter.app"
  lipo -create "target/$arm/release/markdown-converter" "target/$x86/release/markdown-converter" \
    -output "$app/Contents/MacOS/markdown-converter"
  codesign --force --sign - "$app" >/dev/null 2>&1 || true
  rm -f "dist/Markdown Converter-universal.dmg"
  hdiutil create -volname "Markdown Converter" -srcfolder "$app" -ov -format UDZO \
    "dist/Markdown Converter-universal.dmg"
  exit 0
fi

case "$triple" in
  *apple-darwin)  formats="app,dmg" ;;
  *windows*)      formats="nsis" ;;
  *linux*)        formats="deb,appimage" ;;
  *) echo "unsupported target: $triple" >&2; exit 1 ;;
esac

scripts/fetch-pandoc.sh "$triple"
refresh_current "$triple"
# packager reads target/<triple>/release. CARGO_BUILD_TARGET makes its
# before-packaging-command ("cargo build --release") build into that same folder,
# so it finds the build up to date instead of compiling everything a second time.
export CARGO_BUILD_TARGET="$triple"
cargo build --release --target "$triple"
cargo packager --release --target "$triple" --formats "$formats"
