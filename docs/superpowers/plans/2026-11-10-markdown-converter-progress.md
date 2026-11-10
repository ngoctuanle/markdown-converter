# Markdown Converter: Progress

Plan: [2026-11-10-markdown-converter.md](2026-11-10-markdown-converter.md) · Spec: [design](../specs/2026-11-10-markdown-converter-design.md)

**Progress: 12 of 12 tasks done** · Last updated 2026-11-10

| # | Task | Status | Commit | Check |
|---|------|--------|--------|-------|
| 1 | Project scaffold and empty window | ✅ Done | `2155b54` | Builds; window "Markdown Converter" 1100×720 opens, no panic |
| 2 | Input formats (`src/formats.rs`) | ✅ Done | `b158d67` | 3/3 tests |
| 3 | Output-name reservation (`src/naming.rs`) | ✅ Done | `ba6c3a6` | 7/7 tests; concurrency test 50/50 runs |
| 4 | Picture naming (`src/pictures.rs`) | ✅ Done | `0c85028` | 7/7 tests (plan 5 + 2 extra) |
| 5 | Bundled pandoc: fetch script, locate, run with timeout | ✅ Done | `9023fb6` | 4/4 tests; pandoc 3.13 downloaded, checksum verified |
| 6 | Picture rewrite on the pandoc JSON AST (`src/media.rs`) | ✅ Done | `610be3f` | 7/7 tests (plan 6 + 1 extra); checked against real pandoc output |
| 7 | One-file conversion pipeline + fixtures | ✅ Done | `e8610eb` | 11/11 integration tests; full suite 28 + 11 pass, no warnings |
| 8 | Parallel worker pool (`src/queue.rs`) | ✅ Done | `187918b`, `047b3de` | 4/4 tests (3 + panic recovery); full suite 32 + 11 pass, no warnings |
| 9 | File-list model and settings | ✅ Done | `80035cf` | 8/8 tests (6 file list + 2 settings); full suite pass, no warnings |
| 10 | Main window: top bar, file list, drag-and-drop, conversion | ✅ Done | `2800985` | Build clean (no warnings, clippy clean in `src/ui`); launches and stays up 8 s with no stderr, also with pandoc missing; full suite 43 + 11 pass. Still needs a hand check: drag-drop highlight, file dialog, converting the fixtures, settings remembered |
| 11 | Preview pane | ✅ Done | `e3173e9`, `7b406c9` | 2/2 plan tests + 2 helper tests (percent-decoding, relative links) + 1 extra row-wording test; full suite 49 + 11 pass, build has no warnings; launches and stays up 8 s with no stderr. Checked outside the GUI: `with-pictures.docx` converted as `report.docx` and as `Báo cáo Q3 (final).docx`; the placeholder pass leaves the PNG links unchanged and every link (including the `<…>`-wrapped ones) resolves to an existing file. Still needs a hand check: Markdown mode with line numbers, Preview with the pictures, Show in folder, failed-file error text |
| 12 | Packaging with the bundled pandoc | ✅ Done | (this commit) | `scripts/package.sh aarch64-apple-darwin` builds `.app` and `.dmg`; Resources hold `pandoc` (3.13), `pandoc-COPYING.md`, `THIRD_PARTY_NOTICES.md`. A release probe run from inside the bundle with `PATH=/usr/bin:/bin` located `Contents/MacOS/../Resources/pandoc` and converted Markdown to HTML. The real app launched from the bundle for 8 s with no stderr. Full suite passes. Still needs a hand check: open the `.app`, convert `tests/fixtures/two-sheets.xlsx`, confirm Done and no "pandoc not found" notification. The universal, Windows and Linux paths were not run |
| — | Whole-project review and fixes | ⬜ Next | | After Task 12 |

Status key: ✅ Done · 🔄 In progress · ⬜ To do · ❌ Blocked

## Milestones

- **Core logic** (Tasks 2–4): no UI, no pandoc. ✅ Done.
- **Conversion working end to end** (Tasks 5–7): real files convert to Markdown with pictures. ✅ Done.
- **Usable app** (Tasks 8–11): drag files in, convert in parallel, preview the result. ✅ Done (pending hand check).
- **Shippable** (Task 12): a packaged macOS app with pandoc inside. ✅ Done (macOS arm64 verified; Windows, Linux and universal not verified here).

## Decisions made during the work

- **Task 1:** The folder had no git history, so the existing spec and plan were committed first (`a1636d8`) as the starting point.
- **Task 3:** The reservation type also derives `Debug`, which the plan didn't list, so later code can unwrap results that hold one. It only adds a capability.
- **Task 4:** Two extra tests and fixes beyond the plan. (1) A name made by deduplication is never handed out again, so alt texts "Logo", "Logo", "Logo 2" give `logo`, `logo-2`, `logo-2-2` instead of two `logo-2` files overwriting each other. (2) Alt text that slugs to a Windows device name (`con`, `nul`, `aux`, `prn`, `com1`–`9`, `lpt1`–`9`) falls back to `image-NNN`, because Windows can't create those files.
- **Task 5:** Small additions to the plan. A time-out shorter than a minute shows seconds instead of "0 min". If pandoc is killed by a signal with no error text, the row says so. When pandoc times out, the app stops waiting at once instead of waiting for the output pipes to close.
- **Task 6:** The plan didn't say what the `image-NNN` number counts. It counts each new picture copied, in document order: a picture used twice keeps its first name, and web links don't use up a number, so numbered names have no gaps. Picture links written with `%20`-style escapes are also found.
- **Task 8:** If a conversion crashes inside the app (a bug, not a pandoc error), its row is marked Failed with "Internal error while converting" and the worker keeps going.
- **Task 10:** The window has a minimum size of 800×500 so both the file list and the preview stay usable. API notes: the "Add files" dialog result is handled with `cx.spawn_in` rather than `cx.spawn`, because adding files can show a notification, which needs the window. The "pandoc not found" notification is pushed with `window.defer`, because the window's notification layer doesn't exist yet while the view is being built. Dialog errors and settings-save errors show as error notifications.
- **Task 11:** The plan's `set_readonly(true, cx)` on the editor state doesn't work in gpui-component 0.9.1: the editor element copies its own read-only flag (default off) into the state every time it draws. The Markdown view is made read-only with `Editor::readonly(true)` on the element instead, which still allows selecting and copying. The Done-row summary now says "1 image" and "1 warning" for one, and "{n} images" and "{k} warnings" otherwise. The preview doesn't read the file again when the same converted file is shown again (the window re-sends the selection on every conversion update).
- **Task 12:** `before-packaging-command = "cargo build --release"` builds for the host only, but packager looks in `target/<triple>/release`, so `scripts/package.sh` also runs `cargo build --release --target <triple>` first. `locate()` needed no change: packager puts resources in `Contents/Resources`. The universal path (not run) packages the `.app` from the arm64 build, `lipo`s the two binaries into it, ad-hoc signs, and makes the `.dmg` with `hdiutil`.

## Review Focus (from the plan): where each is covered

1. Non-ASCII names, spaces and parentheses in file names: ✅ Task 7
2. Same-name files converting into one folder at the same moment: ✅ Task 3
3. Output folder missing or read-only: ✅ Task 3 (read-only); ✅ Task 7 (missing)
4. PowerPoint alt text that is a file name, and titles like "Picture 1": ✅ Task 4 (unit); ✅ Task 7 (real pptx)
5. Source file deleted before converting: ✅ Task 7
