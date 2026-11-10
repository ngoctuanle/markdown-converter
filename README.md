# Markdown Converter

A desktop app that turns office and document files into clean, GitHub-flavoured Markdown for AI agents. Drop in many files, press **Convert**, and each one gets a `-converted.md` file next to it, with its pictures in a matching `-assets` folder.

- Inputs: `.docx .pptx .xlsx .odt .fodt .rtf .epub .html .htm`. Legacy `.doc/.xls/.ppt` aren't supported.
- Engine: [pandoc](https://pandoc.org) 3.13. It's bundled with the app and runs as a separate process with `--sandbox`, so nothing is ever fetched from the network.
- UI: Rust + [gpui-kit](https://crates.io/crates/gpui-kit) 0.9.1 (Zed's GPUI and gpui-component).
- Design: [spec](docs/superpowers/specs/2026-11-10-markdown-converter-design.md) · [plan](docs/superpowers/plans/2026-11-10-markdown-converter.md) · [progress](docs/superpowers/plans/2026-11-10-markdown-converter-progress.md)

## Prerequisites

| | |
|---|---|
| Rust | 1.99 or newer (edition 2024): `rustc --version` |
| curl, unzip | used by `scripts/fetch-pandoc.sh` |
| macOS | Xcode command-line tools: `xcode-select --install` |
| Linux | `gcc g++ clang libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev libasound2-dev libvulkan1` |
| Windows | Visual Studio 2022 with the "Desktop development with C++" workload, and Git Bash to run the scripts |
| Packaging only | `cargo install cargo-packager --locked --version 0.11.8` |
| Regenerating test fixtures only | `python3` |

## Quick start

```bash
# 1. Download pandoc 3.13 for your machine into vendor/pandoc/<target-triple>/
#    (checks the SHA-256; safe to re-run, it skips if already present)
scripts/fetch-pandoc.sh

# 2. Run the app (debug build)
cargo run
```

A window titled **Markdown Converter** opens. Add files with **Add files** or by dropping them (or a folder) onto the list, then press **Convert N**. A dropped folder adds its supported files, skipping Office lock files (`~$x.docx`) and hidden files such as macOS `._x.docx`.

Click a converted file to see it in the preview pane: raw Markdown or formatted. Any warnings from its conversion (for example a picture link that couldn't be resolved) are listed above it. The preview loads at most the first 2 MB of a file; use **Show in folder** for the rest. Closing the window (or **Quit**, cmd-q / ctrl-q) quits the app. Conversions still running are stopped and their partial output is deleted.

Output for `report.docx`:

```
report-converted.md        ← the Markdown
report-assets/             ← only created if the document has pictures
  sales-chart-q3.png       ← named from the picture's alt text
  image-002.png            ← numbered when there's no usable alt text
```

Pictures embedded in the document (including `data:` pictures in HTML) are saved into the assets folder. A linked local picture is copied only if it is a picture file (`png jpg jpeg gif bmp tif tiff emf wmf svg webp`) and not on a network share (`\\host\share`, `//host/…`). Otherwise the link is left as it is and the row shows a warning. Remote (`http(s)`) pictures stay as links. Tables too complex for Markdown pipe tables are kept as HTML `<table>`s. That is the only HTML in the output.

Existing output is never overwritten. A second run gives `report-converted (1).md` and `report-assets-(1)/`. To send all output to one folder, tick **Using fixed output location** and pick the folder. The setting is remembered.

## Running the tests

The tests run the real pandoc, so do step 1 of the quick start first.

```bash
cargo test                      # everything: unit tests + integration tests
cargo test --lib                # unit tests only (fast; no GUI)
cargo test --test convert       # end-to-end conversions of the files in tests/fixtures/
cargo test --lib naming         # one module: formats, naming, pictures, pandoc, ast, media,
                                #   queue, file_list, settings, preview_text, ui
cargo test --test convert docx_with_pictures -- --nocapture   # one test, show its output
```

Where the tests live:

| Area | Tests |
|---|---|
| Output names, `(n)` numbering, concurrent reservation | `src/naming.rs` |
| Picture names (alt text, `image-NNN`, de-duplication) | `src/pictures.rs` |
| Finding and running pandoc, timeout | `src/pandoc.rs` |
| Picture copy/rename/relink in the pandoc AST, linked-picture rules, `data:` pictures | `src/media.rs` |
| AST clean-up before writing Markdown (no HTML except complex tables) | `src/ast.rs` |
| Worker pool (parallelism, cancel, panic recovery) | `src/queue.rs` |
| File list and settings | `src/file_list.rs`, `src/settings.rs` |
| Whole-file conversions (docx, pptx, xlsx, odt, rtf, epub, html, complex table, corrupt file, Unicode and `#`/`%` names, clean-up after a late failure) | `tests/convert.rs` |

The binary fixtures in `tests/fixtures/` are committed. To regenerate them (needs pandoc and python3 on `PATH`):

```bash
PATH="$PWD/vendor/pandoc/$(rustc -vV | sed -n 's/host: //p'):$PATH" tests/fixtures/make_fixtures.sh
```

## Debugging

### Which pandoc is being used?

`Pandoc::locate()` ([src/pandoc.rs](src/pandoc.rs)) takes the first of these that exists:

1. The `MDCONV_PANDOC` environment variable.
2. Next to the app's executable: `pandoc`/`pandoc.exe`, `../Resources/pandoc` (macOS `.app`), `../lib/markdown-converter/pandoc` (Linux `.deb`/AppImage), `resources/pandoc`.
3. **Debug builds only:** `vendor/pandoc/<target-triple>/pandoc`, then `pandoc` on your `PATH`.

If none is found, the app shows "Bundled pandoc not found — reinstall the app" and **Convert** stays disabled. To try another pandoc:

```bash
MDCONV_PANDOC=/opt/homebrew/bin/pandoc cargo run
```

### A file fails to convert

Click the failed row. The preview pane shows pandoc's full error text, which you can select and copy. To reproduce outside the app, run the same two pandoc steps the app runs ([src/convert.rs](src/convert.rs)):

```bash
P=vendor/pandoc/aarch64-apple-darwin/pandoc   # your triple
# 1. Document → JSON AST, extracting pictures (use the -f reader for the file type)
$P --sandbox -f docx -t json --extract-media=/tmp/media "My file.docx" > /tmp/ast.json
# 2. JSON AST → Markdown
$P --sandbox -f json -t gfm --wrap=none < /tmp/ast.json
```

In between those two steps the app cleans up the AST so the only HTML left is complex tables ([src/ast.rs](src/ast.rs)), and renames and relinks the pictures ([src/media.rs](src/media.rs)). Step 2 on the raw AST can therefore show HTML (`<figure>`, `<span>`, `<img …>`) that the app's output doesn't have. Each file has a 5-minute time-out, after which the row shows "Timed out after 5 min".

### Panics and backtraces

```bash
RUST_BACKTRACE=1 cargo run
```

A panic inside a conversion doesn't stop the app. That row is marked Failed with "Internal error while converting", the panic message is shown in the preview, and the other files keep converting.

### Stepping through in VS Code

Install the **CodeLLDB** extension and add `.vscode/launch.json`:

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "type": "lldb", "request": "launch", "name": "Debug app",
      "cargo": { "args": ["build", "--bin=markdown-converter"] },
      "env": { "RUST_BACKTRACE": "1" }
    },
    {
      "type": "lldb", "request": "launch", "name": "Debug integration tests",
      "cargo": { "args": ["test", "--no-run", "--test=convert"] },
      "args": ["--nocapture"]
    }
  ]
}
```

From a terminal, `rust-lldb target/debug/markdown-converter` works too.

### Settings

The output-folder setting is plain JSON. Delete it to reset:

| OS | Path |
|---|---|
| macOS | `~/Library/Application Support/markdown-converter/settings.json` |
| Linux | `~/.config/markdown-converter/settings.json` |
| Windows | `%APPDATA%\markdown-converter\settings.json` |

## Packaging

```bash
cargo install cargo-packager --locked --version 0.11.8   # once
scripts/package.sh aarch64-apple-darwin                  # → dist/Markdown Converter.app + .dmg
```

The script downloads and checks pandoc for the target, copies it to `vendor/pandoc/current/`, builds a release binary, and runs `cargo packager`. The package contains `pandoc`, its licence (`pandoc-COPYING.md`) and `THIRD_PARTY_NOTICES.md`.

| Target | Run on | Output |
|---|---|---|
| `aarch64-apple-darwin`, `x86_64-apple-darwin` | macOS | `.app`, `.dmg` |
| `universal-apple-darwin` | macOS | universal `.app` (ad-hoc signed) + `Markdown Converter-universal.dmg` |
| `x86_64-pc-windows-msvc` | Windows | NSIS installer |
| `x86_64-unknown-linux-gnu` | Linux | `.deb`, AppImage |

Only `aarch64-apple-darwin` has been built and checked so far. Code signing and notarisation aren't set up.

Release builds never fall back to a system pandoc. To check that a package uses its bundled copy, run it with a minimal `PATH`:

```bash
PATH=/usr/bin:/bin open "dist/Markdown Converter.app"
```

## Project layout

```
src/
  formats.rs      extension → pandoc reader
  naming.rs       atomic "-converted (n).md" / "-assets-(n)" reservation
  pictures.rs     picture file names from alt text / image-NNN
  pandoc.rs       locate the bundled pandoc, run it with a timeout
  ast.rs          clean the pandoc JSON AST so complex tables are the only HTML
  media.rs        copy, rename and relink pictures in the pandoc JSON AST
  convert.rs      the whole one-file pipeline; cleans up on failure
  queue.rs        worker pool: one thread per CPU core
  file_list.rs    rows and their statuses
  settings.rs     fixed output folder, saved as JSON
  preview_text.rs preview loading (2 MB cap) and placeholders for pictures it can't draw
  ui/             window, top bar, file rows, preview pane
scripts/          fetch-pandoc.sh, package.sh
tests/            integration tests + fixtures
vendor/pandoc/    downloaded pandoc binaries (git-ignored)
```

## Licence notes

pandoc is GPL-2.0-or-later and ships unmodified as a separate program. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
