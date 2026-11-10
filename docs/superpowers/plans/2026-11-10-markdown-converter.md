# Markdown Converter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A cross-platform Rust desktop app (gpui-kit UI) that batch-converts `.docx/.pptx/.xlsx/.odt/.fodt/.rtf/.epub/.html` files to agent-friendly Markdown with a bundled pandoc, extracting pictures into a per-document assets folder.

**Architecture:** A library crate holds every piece of pure logic: formats, output naming, picture naming, the pandoc runner, the JSON-AST picture rewrite, the converter, the worker pool, the file-list model and settings. All of it is unit- and integration-tested without a UI. Each file converts in two pandoc runs: `source → JSON AST` (pictures extracted to a temp dir), a Rust pass that renames and copies pictures and rewrites links, then `JSON → gfm`. A pool of N std threads (N = CPU cores) runs conversions and streams events over a `smol::channel` to the gpui-kit view, which owns a `FileList` model and a preview pane.

**Tech Stack:** Rust (edition 2024, rustc ≥ 1.99), `gpui-kit` 0.9.1 (GPUI plus gpui-component), pandoc 3.13 (bundled binary), `serde`/`serde_json`, `smol` 2, `tempfile`, `wait-timeout`, `deunicode`, `dirs`, `regex`, `cargo-packager` 0.11.

**Spec:** `docs/superpowers/specs/2026-11-10-markdown-converter-design.md`

## Global Constraints

- Accepted extensions are exactly `docx pptx xlsx odt fodt rtf epub html htm` (case-insensitive). The pandoc reader is passed explicitly with `-f`.
- Output names for source stem `S`: index 0 gives `S-converted.md` and `S-assets`. Index n ≥ 1 gives `S-converted (n).md` and `S-assets-(n)`. Use the lowest index where **neither** exists. Never overwrite.
- The assets folder is created only if at least one picture is written. Links are relative: `<assets-dir-name>/<picture-file>`.
- Picture names come from a slug of the alt text, else the title, skipping generic names like "Picture 1". Otherwise use `image-NNN` (1-based document order, 3 digits). Clashes get `-2`, `-3`, …. The original extension is kept and nothing is re-encoded.
- Remote (`http(s)`, any URL scheme) pictures stay as links and are never downloaded. Pandoc always runs with `--sandbox`.
- Markdown output: `-t gfm-raw_html --wrap=none`.
- Pandoc version is pinned to **3.13**. The app finds a bundled binary and never requires a system install in release builds.
- Parallelism: `std::thread::available_parallelism()` workers, min 1, no user setting.
- Per-file timeout: **300 s**, after which pandoc is killed and the row is Failed with "Timed out after 5 min".
- Conversion never runs on the UI thread. One failed file never stops the others. Any partial output of a failed file is removed.
- Settings (`fixed_output`, `fixed_output_dir`) persist at `dirs::config_dir()/markdown-converter/settings.json`.
- On Windows, pandoc is spawned with `CREATE_NO_WINDOW` (`0x08000000`) so no console flashes.
- UI copy, exactly:
  - Buttons: "Add files", "Clear", "Convert {n}", "Markdown", "Preview", "Show in folder", "Retry"
  - Checkbox: "Using fixed output location"
  - Empty list: "Drop files here"
  - Statuses: "Waiting", "Converting", "Done", "Failed"

## Review Focus

1. **Source names with spaces, parentheses or non-ASCII characters** (`Báo cáo Q3 (final).docx`). The `.md` name is correct, and every picture link in it resolves to an existing file. Test: `convert_unicode_name_links_resolve` in Task 7.
2. **Two sources with the same stem converting at the same moment into one folder** (`report.docx` and `report.pptx` with a fixed output). They get different indexes and never share an assets folder. Test: `reserve_concurrent_same_stem_unique` in Task 3.
3. **Output folder not writable or missing.** The row is Failed with a readable reason, the app doesn't crash, and nothing is left behind. Tests: `reserve_readonly_dir_errors` in Task 3 and `convert_missing_output_dir_fails_cleanly` in Task 7.
4. **PowerPoint alt text that is just a file name, and shape titles like "Picture 1".** `red.png` gives `red.png`, not `red-png.png`, and "Picture 1" is never used as a name. Tests: `candidate_strips_image_extension` and `generic_titles_ignored` in Task 4, plus the pptx integration test in Task 7.
5. **A source file deleted or moved between adding it and converting it.** The row is Failed with "File not found", and other rows continue. Test: `convert_missing_source_fails` in Task 7.

---

## File Structure

```
Cargo.toml                 package + deps + [package.metadata.packager]
build.rs                   exports MDCONV_TARGET (target triple) for dev pandoc lookup
.gitignore                 /target, /vendor/pandoc, /dist, /.superpowers
scripts/fetch-pandoc.sh    download + verify + unpack pinned pandoc into vendor/pandoc/<triple>/
scripts/package.sh         fetch pandoc for a target, stage it, run cargo packager
src/main.rs                calls markdown_converter::ui::run()
src/lib.rs                 pub mod declarations
src/formats.rs             InputFormat: extension → pandoc reader
src/naming.rs              output-name reservation (md + assets folder)
src/pictures.rs            slug / generic-name detection / PictureNamer
src/pandoc.rs              locate bundled pandoc, run with stdin + timeout
src/media.rs               walk pandoc JSON AST, copy + rename pictures, rewrite links
src/convert.rs             convert_file(): the whole one-file pipeline
src/queue.rs               Converter worker pool + WorkerEvent stream
src/file_list.rs           FileList model: rows, statuses, add/convert/retry/clear/remove
src/settings.rs            Settings load/save + output_dir_for
src/preview_text.rs        placeholder for pictures the preview can't render
src/ui/mod.rs              run(): app bootstrap, window, theme
src/ui/app_view.rs         AppView: top bar, layout, dialogs, drop, event loop
src/ui/file_rows.rs        rendering of list rows
src/ui/preview.rs          PreviewPane: Markdown|Preview switch, editor, TextView
tests/fixtures/            generated sample files (committed) + generator scripts
tests/convert.rs           integration tests through real pandoc
```

---

### Task 1: Project scaffold and empty window

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `src/main.rs`, `src/lib.rs`, `src/ui/mod.rs`

**Interfaces:**
- Produces:
  - `markdown_converter::ui::run()`, which opens a 1100×720 window titled "Markdown Converter" that follows the system light or dark theme.
  - Crate name `markdown_converter`. Later tasks add `pub mod` lines to `src/lib.rs`.

- [ ] **Step 1: Initialise git and the crate**

```bash
cd /Users/tuanle/repositories/temp/markdown-converter
git init
cargo init --name markdown-converter --edition 2024
```

`.gitignore`:
```
/target
/vendor/pandoc
/dist
/.superpowers
```

- [ ] **Step 2: Write `Cargo.toml` dependencies**

```toml
[dependencies]
gpui-kit = { version = "0.9.1", features = ["tree-sitter-markdown"] }
smol = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tempfile = "3"
wait-timeout = "0.2"
deunicode = "1.6"
dirs = "6"
regex = "1"
anyhow = "1"

[dev-dependencies]
gpui-kit = { version = "0.9.1", features = ["test-support", "tree-sitter-markdown"] }
```
(If `cargo` resolves an older `dirs`, take what resolves. If enabling `test-support` fails with "failed to select a version" for `gpui-fast`, run `cargo update -p gpui-kit`.)

- [ ] **Step 3: Implement `ui::run()` in `src/ui/mod.rs`**

Use the verified bootstrap:
- `gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(|cx| { … })`
- Inside it, call `gpui_kit::init(cx)`, then `gpui_kit::component::Theme::sync_system_appearance(None, cx)`, then `gpui_kit::open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1100.), px(720.)), cx))), titlebar: Some(TitlebarOptions { title: Some("Markdown Converter".into()), ..Default::default() }), ..Default::default() }, cx, |window, cx| cx.new(|cx| AppView::new(window, cx)))`, then `cx.activate(true)`.

For now `AppView` is a placeholder struct in `ui/mod.rs` that renders `div().size_full()`. Its constructor subscribes `cx.observe_window_appearance(window, |_, window, cx| Theme::sync_system_appearance(Some(window), cx))` and stores the `Subscription`. Import component items explicitly, not `gpui_kit::component::*`, because both globs export `Size`. `src/main.rs` contains only `fn main() { markdown_converter::ui::run() }`.

- [ ] **Step 4: Verify it builds and opens**

Run: `cargo build && (cargo run & sleep 8; kill %1)`
Expected: the build succeeds and a window titled "Markdown Converter" appears with no panic in the output.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "chore: scaffold gpui-kit app with empty window"
```

---

### Task 2: Input formats

**Files:**
- Create: `src/formats.rs`
- Modify: `src/lib.rs` (add `pub mod formats;`)
- Test: inline `#[cfg(test)] mod tests` in `src/formats.rs`

**Interfaces:**
- Produces:
  - `pub enum InputFormat { Docx, Pptx, Xlsx, Odt, Fodt, Rtf, Epub, Html }`, deriving `Clone, Copy, Debug, PartialEq, Eq`
  - `impl InputFormat { pub fn from_path(path: &Path) -> Option<Self>; pub fn reader(self) -> &'static str }`
  - `pub const SUPPORTED_EXTENSIONS: &[&str] = &["docx","pptx","xlsx","odt","fodt","rtf","epub","html","htm"];`

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn maps_every_extension_to_reader() {
    let cases = [("a.docx","docx"),("a.pptx","pptx"),("a.xlsx","xlsx"),("a.odt","odt"),
                 ("a.fodt","fodt"),("a.rtf","rtf"),("a.epub","epub"),("a.html","html"),("a.htm","html")];
    for (f, r) in cases { assert_eq!(InputFormat::from_path(Path::new(f)).unwrap().reader(), r, "{f}"); }
}
#[test] fn extension_is_case_insensitive() { assert_eq!(InputFormat::from_path(Path::new("X.DOCX")), Some(InputFormat::Docx)); }
#[test] fn rejects_legacy_and_unknown() {
    for f in ["a.doc","a.xls","a.ppt","a.pdf","a.txt","noext",".docx"] { assert_eq!(InputFormat::from_path(Path::new(f)), None, "{f}"); }
}
```
(`.docx` alone is a hidden file with no stem, so it's rejected.)

- [ ] **Step 2: Run them and confirm they fail**

Run: `cargo test --lib formats`
Expected: FAIL with a compile error, because `InputFormat` isn't defined.

- [ ] **Step 3: Implement `InputFormat` in `src/formats.rs`**

Use `Path::extension` lowercased. Require a non-empty `file_stem`.

- [ ] **Step 4: Run them and confirm they pass**

Run: `cargo test --lib formats`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/formats.rs src/lib.rs && git commit -m "feat: map supported extensions to pandoc readers"
```

---

### Task 3: Output-name reservation

**Files:**
- Create: `src/naming.rs`
- Modify: `src/lib.rs`
- Test: inline tests in `src/naming.rs` (use `tempfile::tempdir()`)

**Interfaces:**
- Produces:
  - `pub fn output_names(stem: &str, index: u32) -> (String, String)`, which returns `(md_file_name, assets_dir_name)`.
  - `pub struct Reservation { pub md_path: PathBuf, pub assets_dir: PathBuf, pub index: u32, file: std::fs::File }`
  - `pub fn reserve_output(out_dir: &Path, stem: &str) -> std::io::Result<Reservation>`
  - `impl Reservation`:
    - `pub fn assets_dir_name(&self) -> String` (the file-name component of `assets_dir`)
    - `pub fn write_markdown(&mut self, bytes: &[u8]) -> std::io::Result<()>`
    - `pub fn discard(self) -> std::io::Result<()>`, which deletes `md_path` and `assets_dir` (recursively) if present

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn names_for_index_zero_and_n() {
    assert_eq!(output_names("report", 0), ("report-converted.md".into(), "report-assets".into()));
    assert_eq!(output_names("report", 1), ("report-converted (1).md".into(), "report-assets-(1)".into()));
    assert_eq!(output_names("Q3 (final)", 2), ("Q3 (final)-converted (2).md".into(), "Q3 (final)-assets-(2)".into()));
}
#[test] fn reserve_first_free_creates_empty_md_only() {
    let d = tempdir().unwrap();
    let r = reserve_output(d.path(), "report").unwrap();
    assert_eq!(r.index, 0);
    assert!(r.md_path.exists());
    assert!(!r.assets_dir.exists());
}
#[test] fn reserve_skips_when_md_exists() {
    let d = tempdir().unwrap();
    std::fs::write(d.path().join("report-converted.md"), "x").unwrap();
    assert_eq!(reserve_output(d.path(), "report").unwrap().index, 1);
}
#[test] fn reserve_skips_when_only_assets_exists() {
    let d = tempdir().unwrap();
    std::fs::create_dir(d.path().join("report-assets")).unwrap();
    std::fs::write(d.path().join("report-converted (1).md"), "x").unwrap();
    assert_eq!(reserve_output(d.path(), "report").unwrap().index, 2);
}
#[test] fn reserve_concurrent_same_stem_unique() {
    let d = tempdir().unwrap();
    let p = d.path().to_path_buf();
    let hs: Vec<_> = (0..16).map(|_| { let p = p.clone(); std::thread::spawn(move || reserve_output(&p, "report").unwrap().index) }).collect();
    let mut idx: Vec<u32> = hs.into_iter().map(|h| h.join().unwrap()).collect();
    idx.sort();
    assert_eq!(idx, (0..16).collect::<Vec<_>>());
}
#[test] fn discard_removes_md_and_assets() {
    let d = tempdir().unwrap();
    let r = reserve_output(d.path(), "a").unwrap();
    std::fs::create_dir(&r.assets_dir).unwrap();
    std::fs::write(r.assets_dir.join("x.png"), "x").unwrap();
    let (md, assets) = (r.md_path.clone(), r.assets_dir.clone());
    r.discard().unwrap();
    assert!(!md.exists() && !assets.exists());
}
#[cfg(unix)] #[test] fn reserve_readonly_dir_errors() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempdir().unwrap();
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    assert!(reserve_output(d.path(), "a").is_err());
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
}
```

- [ ] **Step 2: Run them and confirm they fail**

Run: `cargo test --lib naming`
Expected: FAIL with a compile error.

- [ ] **Step 3: Implement it in `src/naming.rs`**

Loop `index` from 0 upward:
- If `assets_dir` exists, continue.
- Otherwise, `OpenOptions::new().write(true).create_new(true).open(md)`. On `ErrorKind::AlreadyExists`, continue. On any other error, return it.

The exclusive create is the lock. Cap the loop at 10 000 and return an `io::Error` beyond that.

- [ ] **Step 4: Run them and confirm they pass**

Run: `cargo test --lib naming`
Expected: all passed.

- [ ] **Step 5: Commit**

```bash
git add src/naming.rs src/lib.rs && git commit -m "feat: atomic output-name reservation with (n) numbering"
```

---

### Task 4: Picture naming

**Files:**
- Create: `src/pictures.rs`
- Modify: `src/lib.rs`
- Test: inline tests

**Interfaces:**
- Produces:
  - `pub fn slugify(text: &str) -> String`: `deunicode`, lowercase, runs of non-`[a-z0-9]` become `-`, trimmed of `-`, truncated to ≤ 60 chars at a `-` boundary when possible.
  - `pub fn is_generic_name(text: &str) -> bool`: case-insensitive match of `^(picture|image|img|graphic|figure|object|chart|photo)\s*[-_]?\s*\d*$`.
  - `pub fn candidate_name(alt: &str, title: &str) -> Option<String>`: tries `alt`, then `title`. For each, it trims, strips a trailing image extension (`png jpg jpeg gif bmp tif tiff emf wmf svg webp`), skips it if empty or generic, and slugifies. It returns the first non-empty slug.
  - `pub struct PictureNamer` with `pub fn new() -> Self` and `pub fn name(&mut self, alt: &str, title: &str, seq: usize, ext: &str) -> String`. The base is `candidate_name` or `format!("image-{seq:03}")`. If the base is already used, it appends `-2`, `-3`, …. The result is `base` plus `.ext` (ext lowercased; no dot if empty).

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn slugify_basic_and_unicode() {
    assert_eq!(slugify("Sales Chart Q3"), "sales-chart-q3");
    assert_eq!(slugify("Biểu đồ doanh thu"), "bieu-do-doanh-thu");
    assert_eq!(slugify("  --A/B__c  "), "a-b-c");
    assert!(slugify(&"word ".repeat(40)).len() <= 60);
}
#[test] fn generic_titles_ignored() {
    for t in ["Picture 1", "image2", "Graphic 12", "Figure", "IMG_3"] { assert!(is_generic_name(t), "{t}"); }
    assert!(!is_generic_name("Picture of the team"));
    assert_eq!(candidate_name("", "Picture 1"), None);
}
#[test] fn candidate_prefers_alt_then_title() {
    assert_eq!(candidate_name("Sales Chart Q3", "Picture 1").as_deref(), Some("sales-chart-q3"));
    assert_eq!(candidate_name("", "Team photo").as_deref(), Some("team-photo"));
    assert_eq!(candidate_name("   ", "  "), None);
}
#[test] fn candidate_strips_image_extension() { assert_eq!(candidate_name("red.png", "Picture 1").as_deref(), Some("red")); }
#[test] fn namer_sequence_fallback_and_dedup() {
    let mut n = PictureNamer::new();
    assert_eq!(n.name("Logo", "", 1, "PNG"), "logo.png");
    assert_eq!(n.name("Logo", "", 2, "png"), "logo-2.png");
    assert_eq!(n.name("", "", 3, "emf"), "image-003.emf");
    assert_eq!(n.name("", "", 4, ""), "image-004");
}
```

- [ ] **Step 2: Run them and confirm they fail**

Run: `cargo test --lib pictures`
Expected: FAIL with a compile error.

- [ ] **Step 3: Implement it in `src/pictures.rs`**

Signatures as in Interfaces. Use `regex::Regex` in a `std::sync::LazyLock` for the generic pattern.

- [ ] **Step 4: Run them and confirm they pass**

Run: `cargo test --lib pictures`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add src/pictures.rs src/lib.rs && git commit -m "feat: picture naming from alt text with sequence fallback"
```

---

### Task 5: Bundled pandoc: fetch script, locate, run with timeout

**Files:**
- Create: `scripts/fetch-pandoc.sh`, `build.rs`, `src/pandoc.rs`
- Modify: `src/lib.rs`
- Test: inline tests in `src/pandoc.rs`

**Interfaces:**
- Produces:
  - `pub const PINNED_VERSION: &str = "3.13";`
  - `pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);`
  - `pub struct Pandoc { path: PathBuf, timeout: Duration }`, deriving `Clone, Debug`
  - `impl Pandoc`:
    - `pub fn locate() -> Result<Pandoc, PandocError>`
    - `pub fn at(path: PathBuf) -> Pandoc`
    - `pub fn with_timeout(self, t: Duration) -> Pandoc`
    - `pub fn path(&self) -> &Path`
    - `pub fn run(&self, args: &[OsString], stdin: Option<Vec<u8>>) -> Result<Vec<u8>, PandocError>`, which returns stdout
  - `pub enum PandocError { NotFound, Spawn(std::io::Error), Timeout(Duration), Failed { code: Option<i32>, stderr: String } }`
  - `impl PandocError`:
    - `pub fn short_reason(&self) -> String`
    - `pub fn detail(&self) -> String`

- [ ] **Step 1: Write `scripts/fetch-pandoc.sh`**

Usage: `scripts/fetch-pandoc.sh [target-triple]`. The default is the host triple from `rustc -vV`. It downloads `https://github.com/jgm/pandoc/releases/download/3.13/<asset>`, checks the SHA-256 (with `shasum -a 256` or `sha256sum`; any mismatch exits non-zero), unpacks into a temp dir, finds the file named `pandoc` (`pandoc.exe` on Windows) anywhere inside, and installs it to `vendor/pandoc/<triple>/` with mode 755. It also downloads `https://raw.githubusercontent.com/jgm/pandoc/3.13/COPYING.md` to `vendor/pandoc/<triple>/COPYING.md`. If the binary is already present and `--version` reports 3.13, it skips everything. Pinned values:

| triple | asset | sha256 |
|---|---|---|
| aarch64-apple-darwin | pandoc-3.13-arm64-macOS.zip | f436d18bebbfb3ddd326597686dacab9fac5dc23bd060495020f35a28a43a7c3 |
| x86_64-apple-darwin | pandoc-3.13-x86_64-macOS.zip | 1823278715aa405aaf21b59cde5dc789346aa8d4fc307467f9a9ee84162bedc5 |
| x86_64-pc-windows-msvc | pandoc-3.13-windows-x86_64.zip | 23e1f2e978a3df80db8bf65c9c59534f41a23140480cfb4f783a882ab7dc6a4e |
| x86_64-unknown-linux-gnu | pandoc-3.13-linux-amd64.tar.gz | 38aa33b325a2bc96a4c39409631212db55702f564250bcdbead180b3bf646389 |

`universal-apple-darwin`: fetch both macOS triples, then `lipo -create -output vendor/pandoc/universal-apple-darwin/pandoc <arm64> <x86_64>`.

Run: `scripts/fetch-pandoc.sh && vendor/pandoc/aarch64-apple-darwin/pandoc --version | head -1`
Expected: `pandoc 3.13`

- [ ] **Step 2: Write `build.rs`**

`println!("cargo:rustc-env=MDCONV_TARGET={}", std::env::var("TARGET").unwrap());`

- [ ] **Step 3: Write the failing tests**

```rust
#[test] fn locate_finds_a_pandoc_in_dev() {
    let p = Pandoc::locate().expect("run scripts/fetch-pandoc.sh first");
    let out = p.run(&["--version".into()], None).unwrap();
    assert!(String::from_utf8_lossy(&out).starts_with(&format!("pandoc {PINNED_VERSION}")));
}
#[test] fn run_pipes_stdin_to_stdout() {
    let p = Pandoc::locate().unwrap();
    let out = p.run(&["-f".into(), "markdown".into(), "-t".into(), "gfm".into()], Some(b"# Hi\n".to_vec())).unwrap();
    assert_eq!(String::from_utf8(out).unwrap().trim(), "# Hi");
}
#[test] fn failure_reports_first_stderr_line() {
    let p = Pandoc::locate().unwrap();
    let e = p.run(&["-f".into(), "nonsense-format".into()], Some(vec![])).unwrap_err();
    assert!(matches!(e, PandocError::Failed { .. }));
    assert!(!e.short_reason().is_empty() && e.short_reason().len() <= 120);
}
#[cfg(unix)] #[test] fn timeout_kills_process() {
    let d = tempfile::tempdir().unwrap();
    let fake = d.path().join("pandoc");
    std::fs::write(&fake, "#!/bin/sh\nsleep 30\n").unwrap();
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let t = std::time::Instant::now();
    let e = Pandoc::at(fake).with_timeout(Duration::from_millis(300)).run(&[], None).unwrap_err();
    assert!(matches!(e, PandocError::Timeout(_)));
    assert!(t.elapsed() < Duration::from_secs(5));
    assert_eq!(PandocError::Timeout(DEFAULT_TIMEOUT).short_reason(), "Timed out after 5 min");
}
```

- [ ] **Step 4: Run them and confirm they fail**

Run: `cargo test --lib pandoc`
Expected: FAIL with a compile error.

- [ ] **Step 5: Implement it in `src/pandoc.rs`**

`locate()` returns the first existing candidate, in this order:
1. The env var `MDCONV_PANDOC`.
2. `exe_dir/pandoc[.exe]`.
3. `exe_dir/../Resources/pandoc` (macOS bundle).
4. `exe_dir/../lib/markdown-converter/pandoc` (Linux deb).
5. `exe_dir/resources/pandoc[.exe]`.
6. Only under `cfg(debug_assertions)`: `concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/pandoc/", env!("MDCONV_TARGET"))/pandoc[.exe]`, then bare `pandoc` resolved from `PATH`. Check the PATH entry by running `--version` successfully.

If nothing is found, return `NotFound`.

`run()`:
- Spawn with stdin, stdout and stderr all piped. On Windows, add `CommandExt::creation_flags(0x08000000)`.
- Write stdin on its own thread, then drop it.
- Drain stdout and stderr on two threads.
- Call `wait_timeout::ChildExt::wait_timeout(self.timeout)`. On `None`, kill, wait, and return `Timeout`.
- A non-zero exit returns `Failed { code, stderr }`.

`short_reason()`:
- `NotFound`: "Bundled pandoc not found"
- `Spawn`: "Could not start pandoc: {e}"
- `Timeout(d)`: "Timed out after {d.as_secs()/60} min"
- `Failed`: the first non-empty stderr line, truncated to 120 chars, or "pandoc exited with code {code}" if stderr is empty

`detail()` returns the full stderr, or the same string as `short_reason`.

- [ ] **Step 6: Run them and confirm they pass**

Run: `cargo test --lib pandoc`
Expected: 4 passed (3 on Windows).

- [ ] **Step 7: Commit**

```bash
git add scripts/fetch-pandoc.sh build.rs src/pandoc.rs src/lib.rs && git commit -m "feat: locate bundled pandoc and run it with a timeout"
```

---

### Task 6: Picture rewrite on the pandoc JSON AST

**Files:**
- Create: `src/media.rs`
- Modify: `src/lib.rs`
- Test: inline tests (build small AST `serde_json::Value`s by hand with `json!`)

**Interfaces:**
- Consumes: `pictures::PictureNamer` (Task 4)
- Produces:
  - `pub struct MediaContext<'a> { pub extract_dir: &'a Path, pub source_dir: &'a Path, pub assets_dir: &'a Path, pub link_prefix: &'a str }`
  - `pub struct MediaReport { pub pictures: usize, pub warnings: Vec<String> }`
  - `pub fn rewrite_images(ast: &mut serde_json::Value, ctx: &MediaContext) -> std::io::Result<MediaReport>`
  - `pub fn inline_text(inlines: &serde_json::Value) -> String`

**Pandoc JSON facts** (verified with 3.13, API `[1,23,1,2]`):
- An image is `{"t":"Image","c":[attr, [inlines…], [src, title]]}`.
- `Str` holds text in `c`. `Space`, `SoftBreak` and `LineBreak` become `" "`. Other inline containers nest inlines in `c`, so recurse.
- With `--extract-media=<dir>`, embedded picture `src` values are `<dir>/…`. Docx gives `<dir>/media/rId10.png`, pptx `<dir>/ppt/media/image1.png`, odt `<dir>/Pictures/0.png`, epub `<dir>/media/file0.png`.
- With `--sandbox`, HTML-linked pictures keep their original `src`.

**Rules,** applied depth-first in document order to every Image. `seq` counts Images 1, 2, 3, … in that order.
1. Normalise `src` and `extract_dir` to `/` separators. If `src` starts with the extract dir, the picture is **embedded**: its file is that path.
2. Otherwise, if `src` matches `^[A-Za-z][A-Za-z0-9+.-]*:` and is not a Windows drive path `^[A-Za-z]:[\\/]`, it's **remote**: leave it untouched and don't count it.
3. Otherwise it's **local**. Percent-decode it, resolve relative paths against `source_dir`, and treat it as embedded if that file exists. If not, add a warning `Picture not found: {src}` and leave the link unchanged.
4. For a picture file:
   - If this file was already copied earlier in the document, reuse its name.
   - Otherwise, `name = namer.name(inline_text(alt), title, seq, ext)`, `create_dir_all(assets_dir)` lazily, and copy the file. Count it in `pictures`.
   - Set `src = format!("{link_prefix}/{name}")`.

- [ ] **Step 1: Write the failing tests**

Fixture helper: `fn img(alt: &str, src: &str, title: &str) -> Value` builds an Image node, with `alt` as one `Str`. `fn doc(inlines: Vec<Value>) -> Value` is `{"pandoc-api-version":[1,23,1,2],"meta":{},"blocks":[{"t":"Para","c":inlines}]}`. Each test makes a tempdir with `extract/` (holding the PNGs), `src/` and `out/report-assets` paths.

```rust
#[test] fn embedded_renamed_from_alt_and_sequence() {
    // extract/media/rId10.png, extract/media/rId13.png exist
    // doc: img("Sales Chart Q3", "<extract>/media/rId10.png", ""), img("", "<extract>/media/rId13.png", "")
    // rewrite → pictures == 2
    // srcs == ["report-assets/sales-chart-q3.png", "report-assets/image-002.png"]
    // out/report-assets/sales-chart-q3.png and image-002.png exist with the original bytes
}
#[test] fn same_file_twice_copied_once() {
    // two images with the same embedded src → pictures == 1, both srcs equal, one file in assets
}
#[test] fn remote_left_untouched_and_no_assets_dir() {
    // img("x", "https://example.com/a.png", "") → src unchanged, pictures == 0, !assets_dir.exists()
}
#[test] fn local_relative_copied_missing_warned() {
    // src/local.png exists; imgs "local.png" and "missing.png"
    // → first rewritten to "report-assets/local-pic.png" (alt "Local pic")
    // → second unchanged; warnings == ["Picture not found: missing.png"]
}
#[test] fn nested_inlines_found_in_tables_and_figures() {
    // an Image nested inside {"t":"Figure", ...} and inside a Table cell's Plain block is still rewritten
}
#[test] fn inline_text_joins_strs_and_spaces() {
    // [Str "Sales", Space, Strong[Str "Chart"]] → "Sales Chart"
}
```
Write each as real code with the assertions in the comments.

- [ ] **Step 2: Run them and confirm they fail**

Run: `cargo test --lib media`
Expected: FAIL with a compile error.

- [ ] **Step 3: Implement `rewrite_images` and `inline_text` in `src/media.rs`**

Walk recursively over `Value::Array` and `Value::Object`, and act on objects whose `"t" == "Image"`. Keep a `HashMap<PathBuf, String>` from source file to the assigned name. The extension comes from the source file's extension.

- [ ] **Step 4: Run them and confirm they pass**

Run: `cargo test --lib media`
Expected: 6 passed.

- [ ] **Step 5: Commit**

```bash
git add src/media.rs src/lib.rs && git commit -m "feat: copy, rename and relink pictures in pandoc AST"
```

---

### Task 7: One-file conversion pipeline with fixtures

**Files:**
- Create: `src/convert.rs`, `tests/convert.rs`, `tests/fixtures/make_fixtures.sh`, `tests/fixtures/mkxlsx.py`, plus the generated fixtures in `tests/fixtures/`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes:
  - `formats::InputFormat` (Task 2)
  - `naming::reserve_output` / `Reservation` (Task 3)
  - `pandoc::Pandoc` (Task 5)
  - `media::{rewrite_images, MediaContext}` (Task 6)
- Produces:
  - `pub struct ConvertRequest { pub source: PathBuf, pub output_dir: PathBuf }`, deriving `Clone, Debug`
  - `pub struct ConvertOutcome { pub md_path: PathBuf, pub assets_dir: Option<PathBuf>, pub pictures: usize, pub warnings: Vec<String> }`, deriving `Clone, Debug`
  - `pub struct ConvertError { pub reason: String, pub detail: String }`, deriving `Clone, Debug`
  - `pub fn convert_file(pandoc: &Pandoc, req: &ConvertRequest) -> Result<ConvertOutcome, ConvertError>`

**Pipeline:**
1. Get the format, or fail with "Unsupported file type".
2. If the source doesn't exist, fail with "File not found".
3. If `output_dir` isn't a directory, fail with "Output folder not found: {dir}".
4. `reserve_output(output_dir, stem)`. On error: "Cannot write to output folder: {e}".
5. Make `tempfile::TempDir`. Run `pandoc --sandbox -f <reader> -t json --extract-media=<tmp> <source>`.
6. Parse the JSON. `rewrite_images` with `link_prefix = reservation.assets_dir_name()`.
7. Run `pandoc --sandbox -f json -t gfm-raw_html --wrap=none` with the rewritten JSON on stdin.
8. `reservation.write_markdown(stdout)`.

If any step after step 4 fails, call `reservation.discard()` before returning the error. Pandoc errors map to `reason = short_reason()` and `detail = detail()`. `assets_dir` is `Some` only if `pictures > 0`.

- [ ] **Step 1: Add the fixture generators and generate the fixtures**

`tests/fixtures/mkxlsx.py` (verified to produce a two-sheet workbook pandoc reads):
```python
import zipfile,sys
SS={}
def sheet(rows):
    out=[]
    for r,row in enumerate(rows,1):
        cells=''.join(f'<c r="{chr(65+c)}{r}" t="s"><v>{SS.setdefault(v,len(SS))}</v></c>' for c,v in enumerate(row))
        out.append(f'<row r="{r}">{cells}</row>')
    return '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>'+''.join(out)+'</sheetData></worksheet>'
z=zipfile.ZipFile(sys.argv[1],'w')
z.writestr('[Content_Types].xml','<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>')
z.writestr('_rels/.rels','<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>')
z.writestr('xl/workbook.xml','<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Revenue" sheetId="1" r:id="rId1"/><sheet name="Costs" sheetId="2" r:id="rId2"/></sheets></workbook>')
z.writestr('xl/_rels/workbook.xml.rels','<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>')
z.writestr('xl/worksheets/sheet1.xml',sheet([['Region','Revenue'],['North','1200']]))
z.writestr('xl/worksheets/sheet2.xml',sheet([['Item','Cost'],['Rent','500']]))
z.writestr("xl/sharedStrings.xml",'<?xml version="1.0" encoding="UTF-8"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'+"".join(f"<si><t>{k}</t></si>" for k in SS)+"</sst>")
z.close()
```

`tests/fixtures/make_fixtures.sh` (verified):
```bash
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
```

Run: `chmod +x tests/fixtures/make_fixtures.sh && PATH="$PWD/vendor/pandoc/$(rustc -vV | sed -n 's/host: //p'):$PATH" tests/fixtures/make_fixtures.sh && ls tests/fixtures`
Expected: the list includes `corrupt.docx local.png no-pictures.docx page.html sample.epub sample.odt sample.rtf slides.pptx two-sheets.xlsx with-pictures.docx`.

- [ ] **Step 2: Write the failing integration tests in `tests/convert.rs`**

Helper: `fn setup(fixture: &str, as_name: &str) -> (TempDir, PathBuf)` copies `tests/fixtures/<fixture>` into a fresh tempdir under `as_name` (for HTML, it also copies `local.png` next to it) and returns the copy's path. `fn pandoc() -> Pandoc { Pandoc::locate().unwrap() }`. `fn image_srcs(md: &Path) -> Vec<String>` runs `pandoc -f gfm -t json` on the output and collects every Image `src`, using `media::inline_text`-style walking or a simple recursive walk.

```rust
#[test] fn docx_with_pictures() {
    let (d, src) = setup("with-pictures.docx", "report.docx");
    let o = convert_file(&pandoc(), &ConvertRequest { source: src, output_dir: d.path().into() }).unwrap();
    assert_eq!(o.md_path, d.path().join("report-converted.md"));
    assert_eq!(o.pictures, 2);
    let md = std::fs::read_to_string(&o.md_path).unwrap();
    assert!(md.contains("# Quarterly Report") && md.contains("| Region"));
    assert!(md.contains("![Sales Chart Q3](report-assets/sales-chart-q3.png)"));
    assert!(md.contains("report-assets/image-002.png"));
    assert!(!md.contains("<img") && !md.contains("<figure"));
}
#[test] fn docx_without_pictures_has_no_assets_dir() { /* no-pictures.docx → assets_dir None, !d/report-assets exists */ }
#[test] fn second_run_gets_index_one() { /* convert twice → second md "report-converted (1).md"; links use "report-assets-(1)/" */ }
#[test] fn pptx_names_from_alt_not_generic_title() { /* slides.pptx → assets contain "red.png" and "green.png"; no "picture-1*" */ }
#[test] fn xlsx_each_sheet_heading_and_table() { /* md contains "## Revenue", "| North", "## Costs", "| Rent" */ }
#[test] fn odt_rtf_epub_convert_with_pictures() { /* each → Ok, pictures >= 1, every image_srcs entry exists relative to md dir */ }
#[test] fn html_local_copied_remote_kept_missing_warned() {
    /* pictures == 1; md contains "https://example.com/remote.png"; warnings == ["Picture not found: missing.png"] */
}
#[test] fn convert_unicode_name_links_resolve() {
    /* as_name "Báo cáo Q3 (final).docx" → md "Báo cáo Q3 (final)-converted.md";
       every image_srcs(md) entry, joined to md's dir, exists */
}
#[test] fn corrupt_file_fails_and_cleans_up() {
    /* corrupt.docx → Err; reason non-empty; output dir contains no *-converted*.md and no *-assets* */
}
#[test] fn convert_missing_source_fails() { /* nonexistent source → Err reason "File not found" */ }
#[test] fn convert_missing_output_dir_fails_cleanly() { /* output_dir = d/nope → Err reason starts with "Output folder not found" */ }
```
Write each as real code with the assertions in the comments.

- [ ] **Step 3: Run them and confirm they fail**

Run: `cargo test --test convert`
Expected: FAIL with a compile error, because `convert_file` isn't defined.

- [ ] **Step 4: Implement `convert_file` in `src/convert.rs`**

Follow the pipeline above.

- [ ] **Step 5: Run them and confirm they pass**

Run: `cargo test --test convert`
Expected: 11 passed.

- [ ] **Step 6: Commit**

```bash
git add src/convert.rs src/lib.rs tests/ && git commit -m "feat: one-file conversion pipeline with integration tests"
```

---

### Task 8: Parallel worker pool

**Files:**
- Create: `src/queue.rs`
- Modify: `src/lib.rs`
- Test: inline tests (they use a stub conversion function, not pandoc)

**Interfaces:**
- Consumes: `convert::{ConvertRequest, ConvertOutcome, ConvertError, convert_file}`, `pandoc::Pandoc`
- Produces:
  - `pub type JobId = u64;`
  - `pub enum WorkerEvent { Started(JobId), Finished(JobId, Result<ConvertOutcome, ConvertError>) }`
  - `pub type ConvertFn = Arc<dyn Fn(&ConvertRequest) -> Result<ConvertOutcome, ConvertError> + Send + Sync>;`
  - `pub struct Converter`
  - `impl Converter`:
    - `pub fn start(pandoc: Pandoc) -> (Converter, smol::channel::Receiver<WorkerEvent>)`, which uses `default_workers()` and `convert_file`
    - `pub fn start_with(workers: usize, f: ConvertFn) -> (Converter, smol::channel::Receiver<WorkerEvent>)`
    - `pub fn enqueue(&self, id: JobId, req: ConvertRequest)`
    - `pub fn cancel(&self, id: JobId) -> bool`, which is true only if the job was still queued and has been removed
    - `pub fn default_workers() -> usize`
  - `impl Drop for Converter`: sets shutdown and notifies all workers. It does **not** join; workers finish their current job and exit.

- [ ] **Step 1: Write the failing tests**

```rust
fn stub(delay_ms: u64, live: Arc<AtomicUsize>, peak: Arc<AtomicUsize>) -> ConvertFn { /* bump live, record peak, sleep, decrement; return Ok(outcome with md_path = req.source) */ }
#[test] fn runs_at_most_n_at_once_and_all_finish() {
    // start_with(3, stub(50ms)), enqueue 10 → collect 20 events via recv_blocking
    // → peak == 3, every id has Started then Finished
}
#[test] fn cancel_removes_queued_job() {
    // start_with(1, stub(200ms)), enqueue ids 1, 2 → wait for Started(1) → cancel(2) == true, cancel(1) == false
    // → only 2 events total (Started 1, Finished 1) within 1 s
}
#[test] fn default_workers_at_least_one() { assert!(Converter::default_workers() >= 1); }
```

- [ ] **Step 2: Run them and confirm they fail**

Run: `cargo test --lib queue`
Expected: FAIL with a compile error.

- [ ] **Step 3: Implement it in `src/queue.rs`**

Shared state is `Arc<(Mutex<State { jobs: VecDeque<(JobId, ConvertRequest)>, shutdown: bool }>, Condvar)>`. Workers are `std::thread`s named `convert-{i}`. Each one loops: wait on the condvar, pop the front, `send_blocking(Started)`, run `f`, `send_blocking(Finished)`. A send error means the UI is gone, so the worker exits. Use an unbounded `smol::channel`.

- [ ] **Step 4: Run them and confirm they pass**

Run: `cargo test --lib queue`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/queue.rs src/lib.rs && git commit -m "feat: worker pool running conversions in parallel"
```

---

### Task 9: File-list model and settings

**Files:**
- Create: `src/file_list.rs`, `src/settings.rs`
- Modify: `src/lib.rs`
- Test: inline tests in both files

**Interfaces:**
- Consumes: `queue::{JobId, WorkerEvent}`, `convert::{ConvertOutcome, ConvertError}`, `formats::InputFormat`
- Produces (`file_list.rs`):
  - `pub enum FileStatus { Waiting, Queued, Converting, Done(ConvertOutcome), Failed(ConvertError) }`
  - `impl FileStatus { pub fn label(&self) -> &'static str }`. `Waiting` and `Queued` both give "Waiting". The others give "Converting", "Done" and "Failed".
  - `pub struct FileItem { pub id: JobId, pub source: PathBuf, pub status: FileStatus }`
  - `pub struct AddReport { pub added: Vec<JobId>, pub duplicates: usize, pub unsupported: Vec<PathBuf> }`
  - `#[derive(Default)] pub struct FileList`
  - `impl FileList`:
    - `pub fn add_paths(&mut self, paths: &[PathBuf]) -> AddReport`
    - `pub fn items(&self) -> &[FileItem]`
    - `pub fn get(&self, id: JobId) -> Option<&FileItem>`
    - `pub fn convertible_count(&self) -> usize`, which counts Waiting and Failed
    - `pub fn is_batch_running(&self) -> bool`, which is true if any item is Queued or Converting
    - `pub fn take_for_convert(&mut self) -> Vec<(JobId, PathBuf)>`, which moves Waiting and Failed to Queued
    - `pub fn queue_ids(&mut self, ids: &[JobId]) -> Vec<(JobId, PathBuf)>`, which moves only Waiting items among `ids` to Queued
    - `pub fn retry(&mut self, id: JobId) -> Option<(JobId, PathBuf)>`, which moves Failed to Queued
    - `pub fn remove(&mut self, id: JobId) -> bool`, which works only for Waiting or Queued
    - `pub fn clear_finished(&mut self)`, which removes Done and Failed
    - `pub fn apply(&mut self, ev: WorkerEvent)`, which ignores unknown ids
- Produces (`settings.rs`):
  - `#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)] pub struct Settings { pub fixed_output: bool, pub fixed_output_dir: Option<PathBuf> }`
  - `impl Settings`:
    - `pub fn default_path() -> Option<PathBuf>`
    - `pub fn load_from(path: &Path) -> Settings`, which returns the default if the file is missing or corrupt
    - `pub fn save_to(&self, path: &Path) -> io::Result<()>`, which creates parent dirs
    - `pub fn output_dir_for(&self, source: &Path) -> PathBuf`

**Rules:**
- `add_paths`:
  - A **directory** adds its direct children that are supported files, sorted by name. Unsupported children are skipped silently.
  - A **file** that's unsupported goes into `unsupported`.
  - A path already in the list (compared after `canonicalize`, or the raw path if that fails) counts as a duplicate.
  - New items start as `Waiting`, with ids increasing from 1.
- **Retry** queues the file at once, so it converts without another press of Convert. This refines the spec's "back to Waiting" so the button visibly does something.

- [ ] **Step 1: Write the failing tests**

```rust
// file_list.rs
#[test] fn add_files_folders_duplicates_unsupported() {
    // tempdir: a.docx, b.pptx, notes.txt, sub/c.docx; add [a.docx, dir, a.docx, x.doc]
    // → added ids for a.docx, b.pptx (from dir; a.docx deduped); duplicates == 2; unsupported == [x.doc]; sub/c.docx not added
}
#[test] fn convert_flow_and_counts() {
    // add 2 → convertible_count 2 → take_for_convert returns 2, both Queued, is_batch_running
    // apply Started(1) → Converting; apply Finished(1, Ok) → Done; Finished(2, Err) → Failed
    // → convertible_count 1, !is_batch_running
}
#[test] fn retry_only_failed() { /* retry on Failed → Some and Queued; on Done → None */ }
#[test] fn remove_only_waiting_or_queued() { /* remove Converting → false; remove Waiting → true */ }
#[test] fn clear_keeps_waiting_and_running() { /* statuses W, Q, C, D, F → after clear: W, Q, C */ }
#[test] fn queue_ids_only_moves_waiting() { /* added during a batch: queue_ids(&report.added) → those become Queued */ }
// settings.rs
#[test] fn output_dir_default_and_fixed() {
    let s = Settings::default();
    assert_eq!(s.output_dir_for(Path::new("/a/b/r.docx")), PathBuf::from("/a/b"));
    let f = Settings { fixed_output: true, fixed_output_dir: Some("/out".into()) };
    assert_eq!(f.output_dir_for(Path::new("/a/b/r.docx")), PathBuf::from("/out"));
    let unset = Settings { fixed_output: true, fixed_output_dir: None };
    assert_eq!(unset.output_dir_for(Path::new("/a/b/r.docx")), PathBuf::from("/a/b"));
}
#[test] fn save_load_roundtrip_and_corrupt_defaults() { /* save→load equal; write "{{garbage" → load == default */ }
```

- [ ] **Step 2: Run them and confirm they fail**

Run: `cargo test --lib -- file_list settings`
Expected: FAIL with a compile error.

- [ ] **Step 3: Implement `FileList` and `Settings`**

Signatures as in Interfaces.

- [ ] **Step 4: Run them and confirm they pass**

Run: `cargo test --lib`
Expected: all lib tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/file_list.rs src/settings.rs src/lib.rs && git commit -m "feat: file-list model and persisted output settings"
```

---

### Task 10: Main window: top bar, file list, drag-and-drop, conversion loop

**Files:**
- Create: `src/ui/app_view.rs`, `src/ui/file_rows.rs`
- Modify: `src/ui/mod.rs` (remove the placeholder and use `AppView` from `app_view.rs`)

**Interfaces:**
- Consumes:
  - `FileList` and `Settings` (Task 9)
  - `Converter` and `WorkerEvent` (Task 8)
  - `Pandoc::locate` (Task 5)
  - `formats::SUPPORTED_EXTENSIONS`
- Produces:
  - `pub struct AppView { files: FileList, converter: Option<Converter>, settings: Settings, settings_path: Option<PathBuf>, selected: Option<JobId>, preview: Entity<PreviewPane>, _subs: Vec<Subscription>, _event_task: Option<Task<()>> }`
  - `impl AppView { pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self }`
  - `PreviewPane` is a stub entity here, rendering an empty div, and is filled in by Task 11. Signature: `PreviewPane::new(window, cx)` and `pub fn show(&mut self, item: Option<&FileItem>, window: &mut Window, cx: &mut Context<Self>)`.

**Behaviour (verified gpui-kit APIs in parentheses):**
- **Startup:**
  - Call `Pandoc::locate()`. On error, keep `converter = None` and push a persistent `Notification::error("Bundled pandoc not found — reinstall the app")` with `.autohide(false)` (`window.push_notification`). Convert stays disabled.
  - On success, call `Converter::start(pandoc)`. Spawn the event loop: `cx.spawn_in(window, async move |this, cx| while let Ok(ev) = rx.recv().await { this.update_in(cx, |this, window, cx| { this.files.apply(ev); this.refresh_preview(window, cx); cx.notify(); }).ok(); })` and store the Task.
- Settings are loaded from `Settings::default_path()`.
- **Top bar** (`h_flex()`):
  - "Add files" button (`Button::new("add").icon(IconName::Plus).label("Add files")`): `cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: None })`. Await it in `cx.spawn` and pass the result to `add_paths`. The dialog can't filter by type, so filtering happens in `add_paths`.
  - "Clear" button: `files.clear_finished()`. If the selected row is gone, clear the selection.
  - Spacer.
  - "Using fixed output location" `Checkbox`:
    - When checked without a folder set, it opens a folder dialog (`files: false, directories: true, multiple: false`). If the dialog is cancelled, the box stays unchecked.
    - When unchecked, it keeps the folder for next time.
    - Save settings on every change.
  - A folder button showing the folder's last path component, or "Choose folder…". Its tooltip is the full path. It's disabled while the checkbox is unchecked, and clicking it re-picks the folder.
  - "Convert {n}" primary button, where `n = files.convertible_count()`. It's disabled if `n == 0` or there's no converter. On click: for each `(id, path)` in `files.take_for_convert()`, call `converter.enqueue(id, ConvertRequest { source: path, output_dir: settings.output_dir_for(&path) })`.
- **`add_paths(paths)`:**
  - `let running = files.is_batch_running(); let r = files.add_paths(&paths);`
  - If `running`, enqueue `files.queue_ids(&r.added)`.
  - If `r.unsupported` isn't empty, push `Notification::warning(format!("Skipped {} unsupported file(s): {}", n, names))`, listing names joined with ", " and truncated to 5 with "…".
- **Body:** `h_resizable("main").child(resizable_panel().size(px(380.)).child(file list)).child(resizable_panel().child(self.preview.clone()))`.
- **File list** (`file_rows.rs`, a `div().id("file-list").size_full().overflow_y_scrollbar()`):
  - It is the drop target: `.drag_over::<ExternalPaths>(|s, _, _, cx| s.border_color(cx.theme().primary).bg(cx.theme().accent))` and `.on_drop(cx.listener(|this, p: &ExternalPaths, window, cx| this.add_paths(p.paths().to_vec(), window, cx)))`. Put the same `on_drop` on the root div, so dropping anywhere in the window works.
  - When empty, it shows a centred "Drop files here" label.
  - **Row:** source file name, then the status label on the right, coloured: Done is success, Failed is danger, Converting has a `Spinner`, Waiting is muted.
    - A second muted line for Done: `→ {md file name} · {n} images`, adding ` · {k} warnings` when present.
    - A second line for Failed: `reason`.
    - A Waiting or Queued row has a small ghost "×" button. It calls `converter.cancel(id)` for Queued items, then `files.remove(id)` if the cancel succeeded or the item was Waiting.
    - A Failed row has a "Retry" ghost button: `files.retry(id)` and then enqueue.
    - Clicking a row sets `selected` and calls `preview.update(cx, |p, cx| p.show(item, window, cx))`. The selected row uses the theme's accent background.

- [ ] **Step 1: Implement `AppView` and the rows as described**

Keep `refresh_preview` as a method that re-sends the selected item to the preview, so a row selected while converting updates when it finishes.

- [ ] **Step 2: Build**

Run: `cargo build`
Expected: success with no warnings in `src/ui`.

- [ ] **Step 3: Manual check (record results in the commit message body)**

Run: `cargo run`. Then:
1. Click Add files and pick `tests/fixtures/with-pictures.docx`, `slides.pptx`, `two-sheets.xlsx`, `corrupt.docx` and `local.png`. Expected: 4 rows, plus a warning that 1 unsupported file (`local.png`) was skipped.
2. Drag `tests/fixtures/sample.odt` onto the list. Expected: the list highlights while hovering, and a row is added.
3. Convert 5. Expected: rows go Converting, then Done, and corrupt.docx goes Failed with a reason. Output files appear next to the fixtures. Delete them afterwards.
4. Tick "Using fixed output location", pick a temp folder, Retry the failed row and re-add a converted file. Expected: new output goes to the temp folder. Quit, relaunch, and the checkbox and folder are remembered.

- [ ] **Step 4: Commit**

```bash
git add src/ui && git commit -m "feat: main window with file list, drag-and-drop and parallel conversion"
```

---

### Task 11: Preview pane

**Files:**
- Create: `src/preview_text.rs`, `src/ui/preview.rs` (replacing the Task 10 stub)
- Modify: `src/lib.rs`, `src/ui/mod.rs`
- Test: inline tests in `src/preview_text.rs`

**Interfaces:**
- Consumes: `FileItem` and `FileStatus` (Task 9)
- Produces:
  - `pub fn placeholder_unrenderable_images(md: &str) -> String` in `preview_text.rs`
  - `pub enum PreviewMode { Markdown, Preview }`
  - `PreviewPane` with `new` and `show` (signatures from Task 10)

**`placeholder_unrenderable_images` rule:** replace every Markdown image whose target ends (case-insensitively, before an optional `"title"`) in `.emf`, `.wmf`, `.tif`, `.tiff`, `.eps` or `.svgz` with the inline text `` `[picture: <file name>]` ``. The target may be wrapped in `<…>`. Leave everything else untouched. This is used for the formatted Preview only and never written to disk.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn replaces_unrenderable_keeps_others() {
    let md = "a ![Logo](r-assets/logo.emf) b ![x](r-assets/x.png) c ![](<My r-assets/y.WMF> \"t\")";
    assert_eq!(placeholder_unrenderable_images(md),
        "a `[picture: logo.emf]` b ![x](r-assets/x.png) c `[picture: y.WMF]`");
}
#[test] fn no_images_unchanged() { assert_eq!(placeholder_unrenderable_images("# Hi"), "# Hi"); }
```

- [ ] **Step 2: Run them and confirm they fail**

Run: `cargo test --lib preview_text`
Expected: FAIL with a compile error.

- [ ] **Step 3: Implement `placeholder_unrenderable_images`**

Use a `regex` in a `LazyLock`.

- [ ] **Step 4: Run them and confirm they pass**

Run: `cargo test --lib preview_text`
Expected: 2 passed.

- [ ] **Step 5: Implement `PreviewPane` in `src/ui/preview.rs`**

- **State:**
  - `mode: PreviewMode` (default `Markdown`)
  - `editor: Entity<input::EditorState>`, created with `EditorState::new(window, cx).language("markdown").line_number(true)` and then `set_readonly(true, cx)`
  - `current: Option<(PathBuf, String)>` (the md path and its text)
  - `message: Option<String>`
- **`show(item)`:**
  - None selected: `message = "Select a file to preview"`.
  - Waiting or Converting: "Not converted yet".
  - Failed: the error `detail`.
  - Done: read `md_path` with `std::fs::read_to_string`. On error, show "Could not read {path}: {e}". Otherwise set `current` and `editor.set_value(text, window, cx)`.
  - Call `cx.notify()`.
- **Header** (`h_flex()`):
  - `TabBar::new("mode").segmented().selected_index(i).child("Markdown").child("Preview").on_click(...)`, which sets `mode`.
  - Spacer.
  - "Show in folder" button, which calls `cx.reveal_path(&md_path)` and is disabled without `current`.
- **Body:**
  - With `message`: a centred muted label.
  - `Markdown` mode: `input::Editor::new(&self.editor).readonly(true).h_full()`.
  - `Preview` mode: `text::TextView::markdown("md-preview", placeholder_unrenderable_images(&text)).selectable(true).scrollable(true)`. Add `.image_source(move |uri: &SharedUri| -> ImageSource { if is_relative_path(uri) { ImageSource::from(base_dir.join(percent_decode(uri))) } else { uri.clone().into() } })`, where `base_dir` is the md file's parent. Relative means no URL scheme and not absolute.

- [ ] **Step 6: Build and do a manual check**

Run: `cargo build && cargo run`. Convert `tests/fixtures/with-pictures.docx` and select it. Expected:
- Markdown mode shows the raw text in monospace with line numbers.
- Preview shows formatted headings, the table and the red and green pictures inline.
- Show in folder opens Finder with `report-converted.md`, or whatever the copied name is, selected.
- Selecting the failed corrupt file shows pandoc's error text.

Delete the generated outputs afterwards.

- [ ] **Step 7: Commit**

```bash
git add src/preview_text.rs src/ui src/lib.rs && git commit -m "feat: preview pane with raw Markdown and formatted modes"
```

---

### Task 12: Packaging with the bundled pandoc

**Files:**
- Create: `scripts/package.sh`, `THIRD_PARTY_NOTICES.md`
- Modify: `Cargo.toml` (`[package.metadata.packager]`)

**Interfaces:**
- Consumes: `scripts/fetch-pandoc.sh` (Task 5) and the `Pandoc::locate` search order (Task 5)

**Packaging config:**
- `[package.metadata.packager]`:
  - `product-name = "Markdown Converter"`
  - `identifier = "app.markdown-converter"`
  - `before-packaging-command = "cargo build --release"`
  - `out-dir = "dist"`
  - `formats = ["app", "dmg", "nsis", "deb", "appimage"]`. Each OS builds only its own formats; pass `--formats` from the script.
  - `resources = [{ src = "vendor/pandoc/current/pandoc*", target = "." }, { src = "vendor/pandoc/current/COPYING.md", target = "pandoc-COPYING.md" }, "THIRD_PARTY_NOTICES.md"]`
- `scripts/package.sh <triple>`:
  1. Run `scripts/fetch-pandoc.sh <triple>`.
  2. Refresh `vendor/pandoc/current/` from `vendor/pandoc/<triple>/`.
  3. Run `cargo packager --release --target <triple> --formats <per-OS list>`. The lists are: macOS `app,dmg`; Windows `nsis`; Linux `deb,appimage`.

  For `universal-apple-darwin`, build both macOS targets, `lipo` the two app binaries into the `.app`, and use the universal pandoc.
- `THIRD_PARTY_NOTICES.md`: pandoc 3.13 is © John MacFarlane, licensed GPL-2.0-or-later, source at https://github.com/jgm/pandoc/tree/3.13. It is bundled unmodified and runs as a separate program.

- [ ] **Step 1: Write the config, script and notices**

- [ ] **Step 2: Package on macOS and verify the bundled pandoc is found**

Run: `cargo install cargo-packager --locked --version 0.11.8 && scripts/package.sh aarch64-apple-darwin && ls "dist/Markdown Converter.app/Contents/Resources/"`
Expected: the listing shows `pandoc`, `pandoc-COPYING.md` and `THIRD_PARTY_NOTICES.md`.

Run: `PATH=/usr/bin:/bin open "dist/Markdown Converter.app"`, then convert `tests/fixtures/two-sheets.xlsx`.
Expected: Done, with no "pandoc not found" notification. That proves the bundled copy was used and not Homebrew's. If cargo-packager puts resources somewhere `locate()` doesn't search, add that location to `locate()` with a test-free one-line change and note it in the commit.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml scripts/package.sh THIRD_PARTY_NOTICES.md && git commit -m "build: package app with bundled pandoc 3.13"
```

Windows and Linux packages are built on those OSes with the same script. They are out of scope for verification on this Mac. On Linux, install the gpui build dependencies first: `gcc g++ clang libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev libasound2-dev libvulkan1`. On Windows, install VS 2022 with the C++ desktop workload.
