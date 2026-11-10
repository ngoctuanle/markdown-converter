#!/usr/bin/env bash
# Download, verify and unpack the pinned pandoc release into vendor/pandoc/<triple>/.
# Usage: scripts/fetch-pandoc.sh [target-triple]   (default: host triple from rustc)
set -euo pipefail

VERSION="3.13"
root="$(cd "$(dirname "$0")/.." && pwd)"
triple="${1:-$(rustc -vV | sed -n 's/^host: //p')}"

asset_for() {
  case "$1" in
    aarch64-apple-darwin)     echo "pandoc-$VERSION-arm64-macOS.zip f436d18bebbfb3ddd326597686dacab9fac5dc23bd060495020f35a28a43a7c3" ;;
    x86_64-apple-darwin)      echo "pandoc-$VERSION-x86_64-macOS.zip 1823278715aa405aaf21b59cde5dc789346aa8d4fc307467f9a9ee84162bedc5" ;;
    x86_64-pc-windows-msvc)   echo "pandoc-$VERSION-windows-x86_64.zip 23e1f2e978a3df80db8bf65c9c59534f41a23140480cfb4f783a882ab7dc6a4e" ;;
    x86_64-unknown-linux-gnu) echo "pandoc-$VERSION-linux-amd64.tar.gz 38aa33b325a2bc96a4c39409631212db55702f564250bcdbead180b3bf646389" ;;
    *) return 1 ;;
  esac
}

sha256() {
  if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d' ' -f1
  else sha256sum "$1" | cut -d' ' -f1; fi
}

bin_name() { case "$1" in *windows*) echo "pandoc.exe" ;; *) echo "pandoc" ;; esac; }

# True if vendor/pandoc/<triple>/ already holds pandoc $VERSION and its licence.
# A binary for another OS/arch can't be run here, so its presence is enough.
already_have() {
  local dir="$root/vendor/pandoc/$1" bin
  bin="$dir/$(bin_name "$1")"
  [[ -x "$bin" && -f "$dir/COPYING.md" ]] || return 1
  if "$bin" --version >/dev/null 2>&1; then
    [[ "$("$bin" --version | head -1)" == "pandoc $VERSION" ]]
  else
    return 0
  fi
}

fetch_copying() {
  curl -fsSL "https://raw.githubusercontent.com/jgm/pandoc/$VERSION/COPYING.md" -o "$1/COPYING.md"
}

fetch_one() {
  local t="$1" dir spec asset sum work got bin found
  dir="$root/vendor/pandoc/$t"
  if already_have "$t"; then echo "pandoc $VERSION already present for $t"; return 0; fi
  spec="$(asset_for "$t")" || { echo "error: no pinned pandoc asset for target $t" >&2; exit 1; }
  read -r asset sum <<<"$spec"
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' RETURN
  echo "Downloading $asset ..."
  curl -fsSL --retry 3 "https://github.com/jgm/pandoc/releases/download/$VERSION/$asset" -o "$work/$asset"
  got="$(sha256 "$work/$asset")"
  if [[ "$got" != "$sum" ]]; then
    echo "error: SHA-256 mismatch for $asset" >&2
    echo "  expected $sum" >&2
    echo "  got      $got" >&2
    exit 1
  fi
  mkdir -p "$work/x"
  case "$asset" in
    *.zip) unzip -q "$work/$asset" -d "$work/x" ;;
    *.tar.gz) tar -xzf "$work/$asset" -C "$work/x" ;;
  esac
  bin="$(bin_name "$t")"
  found="$(find "$work/x" -type f -name "$bin" | head -1)"
  [[ -n "$found" ]] || { echo "error: $bin not found inside $asset" >&2; exit 1; }
  mkdir -p "$dir"
  install -m 755 "$found" "$dir/$bin"
  fetch_copying "$dir"
  echo "Installed $dir/$bin"
}

if [[ "$triple" == "universal-apple-darwin" ]]; then
  fetch_one aarch64-apple-darwin
  fetch_one x86_64-apple-darwin
  dir="$root/vendor/pandoc/universal-apple-darwin"
  if already_have universal-apple-darwin; then echo "pandoc $VERSION already present for universal-apple-darwin"; exit 0; fi
  mkdir -p "$dir"
  lipo -create -output "$dir/pandoc" \
    "$root/vendor/pandoc/aarch64-apple-darwin/pandoc" \
    "$root/vendor/pandoc/x86_64-apple-darwin/pandoc"
  chmod 755 "$dir/pandoc"
  cp "$root/vendor/pandoc/aarch64-apple-darwin/COPYING.md" "$dir/COPYING.md"
  echo "Installed $dir/pandoc (universal)"
else
  fetch_one "$triple"
fi
