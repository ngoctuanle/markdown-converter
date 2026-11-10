# Markdown Converter: Progress

Plan: [2026-11-10-markdown-converter.md](2026-11-10-markdown-converter.md) · Spec: [design](../specs/2026-11-10-markdown-converter-design.md)

**Progress: 7 of 12 tasks done** · Last updated 2026-11-10

| # | Task | Status | Commit | Check |
|---|------|--------|--------|-------|
| 1 | Project scaffold and empty window | ✅ Done | `2155b54` | Builds; window "Markdown Converter" 1100×720 opens, no panic |
| 2 | Input formats (`src/formats.rs`) | ✅ Done | `b158d67` | 3/3 tests |
| 3 | Output-name reservation (`src/naming.rs`) | ✅ Done | `ba6c3a6` | 7/7 tests; concurrency test 50/50 runs |
| 4 | Picture naming (`src/pictures.rs`) | ✅ Done | `0c85028` | 7/7 tests (plan 5 + 2 extra) |
| 5 | Bundled pandoc: fetch script, locate, run with timeout | ✅ Done | `9023fb6` | 4/4 tests; pandoc 3.13 downloaded, checksum verified |
| 6 | Picture rewrite on the pandoc JSON AST (`src/media.rs`) | ✅ Done | `610be3f` | 7/7 tests (plan 6 + 1 extra); checked against real pandoc output |
| 7 | One-file conversion pipeline + fixtures | ✅ Done | | 11/11 integration tests; full suite 28 + 11 pass, no warnings |
| 8 | Parallel worker pool (`src/queue.rs`) | ⬜ Next | | 3 tests |
| 9 | File-list model and settings | ⬜ To do | | 8 tests |
| 10 | Main window: top bar, file list, drag-and-drop, conversion | ⬜ To do | | Build + manual check |
| 11 | Preview pane | ⬜ To do | | 2 tests + manual check |
| 12 | Packaging with the bundled pandoc | ⬜ To do | | Build a macOS `.app` and convert a file with it |
| — | Whole-project review and fixes | ⬜ To do | | After Task 12 |

Status key: ✅ Done · 🔄 In progress · ⬜ To do · ❌ Blocked

## Milestones

- **Core logic** (Tasks 2–4): no UI, no pandoc. ✅ Done.
- **Conversion working end to end** (Tasks 5–7): real files convert to Markdown with pictures. ✅ Done.
- **Usable app** (Tasks 8–11): drag files in, convert in parallel, preview the result.
- **Shippable** (Task 12): a packaged macOS app with pandoc inside.

## Decisions made during the work

- **Task 1:** The folder had no git history, so the existing spec and plan were committed first (`a1636d8`) as the starting point.
- **Task 3:** The reservation type also derives `Debug`, which the plan didn't list, so later code can unwrap results that hold one. It only adds a capability.
- **Task 4:** Two extra tests and fixes beyond the plan. (1) A name made by deduplication is never handed out again, so alt texts "Logo", "Logo", "Logo 2" give `logo`, `logo-2`, `logo-2-2` instead of two `logo-2` files overwriting each other. (2) Alt text that slugs to a Windows device name (`con`, `nul`, `aux`, `prn`, `com1`–`9`, `lpt1`–`9`) falls back to `image-NNN`, because Windows can't create those files.
- **Task 5:** Small additions to the plan. A time-out shorter than a minute shows seconds instead of "0 min". If pandoc is killed by a signal with no error text, the row says so. When pandoc times out, the app stops waiting at once instead of waiting for the output pipes to close.
- **Task 6:** The plan didn't say what the `image-NNN` number counts. It counts each new picture copied, in document order: a picture used twice keeps its first name, and web links don't use up a number, so numbered names have no gaps. Picture links written with `%20`-style escapes are also found.

## Review Focus (from the plan): where each is covered

1. Non-ASCII names, spaces and parentheses in file names: ✅ Task 7
2. Same-name files converting into one folder at the same moment: ✅ Task 3
3. Output folder missing or read-only: ✅ Task 3 (read-only); ✅ Task 7 (missing)
4. PowerPoint alt text that is a file name, and titles like "Picture 1": ✅ Task 4 (unit); ✅ Task 7 (real pptx)
5. Source file deleted before converting: ✅ Task 7
