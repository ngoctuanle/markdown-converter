#!/usr/bin/env bash
# Regenerates the binary test fixtures. Requires pandoc >= 3.13 and python3 on PATH.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-$here}"
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
python3 -I - "$work" <<'PY'
import sys, zlib, struct
def png(c):
    raw = b''.join(b'\x00' + bytes(c) * 4 for _ in range(4))
    ch = lambda t, d: struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    return b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', 4, 4, 8, 2, 0, 0, 0)) + ch(b'IDAT', zlib.compress(raw)) + ch(b'IEND', b'')
open(sys.argv[1] + '/red.png', 'wb').write(png((255, 0, 0)))
open(sys.argv[1] + '/green.png', 'wb').write(png((0, 255, 0)))
PY
cat > "$work/doc.md" <<'MD'
# Quarterly Report

Some **bold** text.

![Sales Chart Q3](red.png)

![](green.png)

| Region | Revenue |
|--------|---------|
| North  | 1200    |
MD
( cd "$work"
  pandoc doc.md -o "$out/with-pictures.docx"
  printf '# Plain\n\nNo pictures here.\n' | pandoc -f markdown -o "$out/no-pictures.docx"
  pandoc doc.md -o "$out/slides.pptx"
  pandoc doc.md -o "$out/sample.odt"
  pandoc doc.md -o "$out/sample.rtf"
  pandoc doc.md -o "$out/sample.epub" --metadata title=Sample )
python3 -I "$here/mkxlsx.py" "$out/two-sheets.xlsx"
cp "$work/red.png" "$out/local.png"
cat > "$out/page.html" <<'HTML'
<html><body><h1>Web Page</h1>
<p><img src="local.png" alt="Local pic"></p>
<p><img src="https://example.com/remote.png" alt="Remote pic"></p>
<p><img src="missing.png" alt="Missing pic"></p>
</body></html>
HTML
printf 'this is not a zip file' > "$out/corrupt.docx"
