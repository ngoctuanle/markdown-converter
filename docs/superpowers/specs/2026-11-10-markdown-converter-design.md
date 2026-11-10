# Markdown Converter — Design

Date: 2026-11-10
Status: Reviewed, awaiting final approval

## 1. Intent

A cross-platform desktop app that converts office and document files into Markdown (`.md`). **The Markdown is input for AI agents.** That shapes every decision: clean semantic structure (headings, lists, tables, image links) matters more than looking like the original. Page layout, headers and footers, and styling are noise.

The user picks or drops many files at once. The app converts them in parallel using as much of the machine as it sensibly can, saves the `.md` files and their pictures to disk, and lets the user check the result in the app.

## 2. Goals

- Convert many files in one batch, in parallel, as fast as the hardware allows.
- Accept every office and document format **pandoc can read** that is relevant here (see §5).
- Pull out the pictures pandoc extracts from each document into a per-document assets folder, and link them from the Markdown (scope in §6.3).
- Never overwrite existing output.
- Let the user inspect any converted file as raw Markdown or as a formatted preview.
- Run on macOS, Windows and Linux from one Rust codebase.
- Work out of the box: pandoc is bundled, so the user installs nothing else.

## 3. Non-goals

- **Legacy binary formats `.doc`, `.xls`, `.ppt`.** Pandoc can't read them. The user chose to support only what pandoc supports. There is no LibreOffice or other pre-converter.
- Converting *to* anything other than Markdown.
- Showing or rendering the original document (Word or PowerPoint view) inside the app.
- Editing Markdown in the app.
- Faithful visual reproduction: fonts, colours, page layout, slide positioning.
- Cloud or network features of any kind. Conversion is entirely local.

## 4. Anti-goals (failures even if it "works")

- Silently overwriting a file the user already has.
- One bad file stopping or crashing the batch.
- The UI freezing while a batch runs.
- Broken image links in the output, or links that stop working when the `.md` and its assets folder are moved together.
- Requiring the user to install pandoc, or anything else, separately.

## 5. Inputs

**Decided by the user:** accept the formats pandoc lists under "It can convert from" that apply to documents. Office formats are the core; the user agreed to the extra document formats.

| Group | Extensions | pandoc reader |
|---|---|---|
| Office (core) | `.docx` | `docx` |
| | `.pptx` | `pptx` |
| | `.xlsx` | `xlsx` |
| Extra documents | `.odt` | `odt` |
| | `.fodt` | `fodt` |
| | `.rtf` | `rtf` |
| | `.epub` | `epub` |
| | `.html`, `.htm` | `html` |

The reader is chosen from the file extension and passed explicitly (`-f <reader>`). Files with any other extension are rejected at add time with a visible reason, not silently skipped.

*Builder's call:* whether to also accept other pandoc text inputs (`.csv`, `.ipynb`, `.tex`, `.rst`, `.org` and similar). They would work for free, but they aren't "documents" in the user's sense. The default is to leave them out.

**Ways to add files (decided):**
- An "+ Add files" button that opens the native multi-select file dialog, filtered to the extensions above.
- Drag and drop onto the window, **including onto the file list itself**. The list highlights while files are dragged over it and shows "Drop files here" when empty.
- *My call:* dropping a folder adds the supported files directly inside it (not recursively). The builder may make it recursive if that's simple. Adding a file that's already in the list is ignored.

## 6. Output

### 6.1 Location (decided)
- **Default:** the `.md` file goes in the **same folder as its source file**.
- **"Using fixed output location"** (a checkbox plus folder picker): when checked, all output goes to that one folder.
- *My call:* the checkbox state and chosen folder are remembered between launches.

### 6.2 Names (decided)
For a source `report.docx`:

| | Markdown file | Assets folder |
|---|---|---|
| First conversion | `report-converted.md` | `report-assets/` |
| If either already exists | `report-converted (1).md` | `report-assets-(1)/` |
| Next | `report-converted (2).md` | `report-assets-(2)/` |

- The `.md` file and its assets folder **always share the same number**. Pick the lowest `n` for which neither name exists.
- Existing files are **never** overwritten.
- *My call:* numbering starts at `(1)`. I proposed this and the user didn't object.
- The same rule applies when two different sources collide in a fixed output folder (`report.docx` and `report.pptx` both give the stem `report`).
- Because conversions run in parallel, the name must be **reserved atomically**: create the `.md` file exclusively before converting into it, so two workers can't pick the same `n`.

### 6.3 Pictures (decided)
- Every picture **pandoc extracts** from the source is written into the assets folder, next to the `.md`.
- **Scope (decided by the user):** only what pandoc extracts. There is no custom Office extractor. Pictures pandoc skips are accepted as lost in v1:
  - charts and SmartArt
  - pictures placed in spreadsheets
  - pictures in headers and footers
  - PowerPoint master and layout slide images
- **Formats (decided by the user):** pictures keep their original format, including EMF, WMF and TIFF, with no conversion to PNG in v1. The preview shows a placeholder (the file name) for any picture it can't render. PNG conversion is a possible later addition.
- The Markdown links each picture with a **relative path**: `![Sales chart](report-assets/sales-chart.png)`. The `.md` plus its folder can then be moved together without breaking links.
- **Picture file names:**
  1. If the picture has a name or description in the source (in Word or PowerPoint, the picture's alt text/description; otherwise its name or title if available), use a filename-safe slug of it: "Sales Chart Q3" becomes `sales-chart-q3.png`.
  2. Otherwise, number pictures in document order: `image-001.png`, `image-002.png`, …
  3. Name clashes within one document get `-2`, `-3`, … added.
  4. Keep the original file extension and format. No re-encoding.
- If the document has no pictures, **no assets folder is created**.
- **Pictures that are linked, not embedded** (mostly HTML/EPUB, occasionally Word). *My call:*
  - A linked **local** file is copied into the assets folder and renamed by the same rules.
  - A **remote** (`http(s)`) image is left as its original URL. It is never downloaded, because conversion stays fully offline (§3). Run pandoc so it can't fetch from the network, for example with `--sandbox` and the app copying local files itself.
  - A local link that can't be resolved is left as-is and reported as a warning on the row, not a failure.
- *Builder's call:* how to do the renaming. Pandoc's `--extract-media` uses its own names, so the app must rename the files and rewrite the links. Possible approaches: a bundled Lua filter working on pandoc's media bag, or post-processing pandoc's JSON AST in Rust before writing Markdown. Either is fine if the result matches the rules above.

### 6.4 Markdown flavour
*My call:* GitHub-Flavored Markdown (`-t gfm`): pipe tables, no raw HTML where avoidable, no hard line wrapping (`--wrap=none`). This is the most widely understood flavour for AI agents. Tables too complex for pipe syntax may fall back to whatever pandoc emits. The builder should prefer the output an LLM would read most cleanly.

*My call:* spreadsheets come out with **each sheet as a heading followed by a Markdown table**, as pandoc's xlsx reader produces it. Slides come out as pandoc's pptx reader structures them, one section per slide.

## 7. Conversion engine

- **Pandoc is the engine (decided).** It's a native Haskell binary. The app runs it as a **separate process per file** and never links it into the Rust program.
- **Bundled (decided):** the pandoc binary for each platform ships inside the app (the macOS `.app` bundle, alongside the `.exe` on Windows, inside the Linux package). The app finds it relative to its own executable. The cost is a larger download, roughly 100–200 MB per platform.
  - The bundled pandoc version must include the `pptx` and `xlsx` readers. Use a recent 3.x release and pin it.
  - Pandoc is GPL-licensed. Ship its licence and a source notice alongside it. Running it as a separate program keeps the app's own licence independent.
- **Parallel (decided):** run as many conversions at once as the machine has CPU cores, using Rust's available-parallelism value, minimum 1. Each pandoc run is essentially single-threaded, so this keeps every core busy without oversubscribing.
  - *My call:* no user-facing setting for this in v1.
  - Conversion work must never run on the UI thread. The UI stays responsive and status updates live.
- **Errors:** a non-zero exit, a timeout or an unreadable file marks that file **Failed** with a short, human-readable reason from pandoc's stderr. The full text is available when the row is selected. Other files keep converting.
  - Any half-written output for a failed file, including its reserved `.md` and partial assets folder, is cleaned up.
  - *My call:* a per-file timeout, to stop a pathological file from hanging a worker. The builder picks a generous default.
- Pandoc gives no progress-percentage output, so per-file status is only **Waiting → Converting → Done / Failed**.

## 8. User interface (Layout B, chosen by the user)

Built with **gpui-kit** (`github.com/longbridge/gpui-kit`, crate `gpui-kit`, latest at build time, 0.9.x on crates.io as of this writing, on top of Zed's GPUI) and its `gpui-component` layer.

```
┌──────────────────────────────────────────────────────────────────────────┐
│ [+ Add files] [Clear]        ☑ Using fixed output location [~/Docs ▾] [Convert 4] │
├───────────────────────────────┬──────────────────────────────────────────┤
│ Q3-report.docx            ✓   │ [Markdown | Preview]        [Show in folder] │
│  → Q3-report-converted.md·3img│                                          │
│ pitch.pptx         Converting │  # Q3 Report                             │
│ budget.xlsx           Waiting │  ## Summary                              │
│ old.docx                  ✕   │  Revenue grew **12%** …                  │
│  File is damaged              │  ![Sales chart](Q3-report-assets/…)      │
│      (drop files here)        │                                          │
└───────────────────────────────┴──────────────────────────────────────────┘
```

- **Top bar:** Add files, Clear, the "Using fixed output location" checkbox with its folder picker (disabled while unchecked), and **Convert N**, where N is the number of files waiting.
- **Left pane, file list:** one row per file with its name, status, and once done the output filename and picture count. Failed rows show a short reason. The list is a drop target (§5).
  - *My calls:*
    - Files added during a running batch join the queue and convert in that batch.
    - Clear removes finished and failed rows and leaves waiting and running ones.
    - Each row has a small remove (×) for waiting files.
    - **Retry:** a failed row has a Retry action that puts it back to Waiting. Pressing **Convert** also re-queues every failed row, so N counts waiting plus failed files. A retry reuses the same output-name rules, and the failed attempt's partial output was already cleaned up.
- **Right pane, preview** of the selected **converted** file:
  - A two-way switch: **Markdown** shows the raw `.md` text exactly as the agent will read it (read-only, monospace, scrollable). **Preview** shows it formatted with gpui-kit's Markdown renderer, with pictures from the assets folder shown inline.
  - **Show in folder** reveals the output file in Finder, Explorer or the file manager.
  - A placeholder appears when nothing is selected or the selected file isn't converted yet. For a failed file it shows the full error text.
- *My call:* the app follows the system light or dark theme.
- *My call:* no conversion starts until the user presses **Convert**. Adding files only queues them.
- The minimum window size keeps both panes usable. The divider between them is resizable if gpui-kit makes that easy.

## 9. Platforms and packaging

- One Rust codebase, built for **macOS (Apple Silicon and Intel), Windows x64 and Linux x64**. The macOS build should be universal or ship both architectures.
- Each platform package contains the matching pandoc binary.
- *Builder's call:* packaging tooling (for example `cargo-bundle` or `cargo-packager`). Code signing and notarisation are out of scope for v1 but shouldn't be designed out.

## 10. Testing expectations

- Unit tests for naming: the `-converted` suffix, `(n)` numbering, shared numbering between `.md` and assets folder, collisions in a fixed folder, and atomic reservation under concurrency.
- Unit tests for picture naming: slug from description, sequence fallback, duplicate handling, extension kept.
- Integration tests running the bundled pandoc on small fixture files (`docx` with and without pictures, `pptx`, `xlsx` with two sheets, `odt`, `rtf`, `epub`, `html`, and a deliberately corrupt file), checking output text, link paths and assets on disk.
- A manual check on each OS: drag-drop onto the list, file dialog, fixed output folder remembered, preview toggle, Show in folder.

## 11. Left to the builder

- Crate layout, module structure, and the async or threading model behind the worker pool.
- Exactly how pictures are renamed and links rewritten (Lua filter or Rust AST post-processing).
- Which gpui-kit components to use, and any gaps to fill with plain GPUI (for example the native file dialog via GPUI's path prompt, or external file drops via GPUI's external-paths drop events).
- Where settings are stored (the platform's standard config directory).
- The exact pinned pandoc version, and how binaries are fetched into the build.
