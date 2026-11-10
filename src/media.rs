//! Copy, rename and relink the pictures in a pandoc JSON AST.

use crate::pictures::PictureNamer;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Where pictures come from and where they go for one document.
pub struct MediaContext<'a> {
    /// The folder passed to pandoc's `--extract-media`.
    pub extract_dir: &'a Path,
    /// The source document's folder, for resolving relative links.
    pub source_dir: &'a Path,
    /// The assets folder to create and fill (only if a picture is written).
    pub assets_dir: &'a Path,
    /// Prefix for rewritten links: the assets folder's name.
    pub link_prefix: &'a str,
}

#[derive(Debug, Default)]
pub struct MediaReport {
    /// Distinct picture files copied into the assets folder.
    pub pictures: usize,
    pub warnings: Vec<String>,
}

static URL_SCHEME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9+.-]*:").unwrap());
static WINDOWS_DRIVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z]:[\\/]").unwrap());

/// Rewrites every Image in `ast`, in document order: embedded and local
/// pictures are copied into `assets_dir` under readable names and relinked;
/// remote ones are left alone; missing local ones are left alone with a warning.
pub fn rewrite_images(ast: &mut Value, ctx: &MediaContext) -> io::Result<MediaReport> {
    let mut rw = Rewriter {
        ctx,
        extract_prefix: format!(
            "{}/",
            slashes(&ctx.extract_dir.to_string_lossy()).trim_end_matches('/')
        ),
        namer: PictureNamer::new(),
        copied: HashMap::new(),
        report: MediaReport::default(),
    };
    rw.walk(ast)?;
    Ok(rw.report)
}

/// The plain text of a list of pandoc inlines (used for alt text).
pub fn inline_text(inlines: &Value) -> String {
    let mut out = String::new();
    collect_text(inlines, &mut out);
    out
}

fn collect_text(v: &Value, out: &mut String) {
    match v {
        Value::Array(items) => items.iter().for_each(|i| collect_text(i, out)),
        Value::Object(o) => match o.get("t").and_then(Value::as_str) {
            Some("Str") => out.push_str(o.get("c").and_then(Value::as_str).unwrap_or("")),
            Some("Space" | "SoftBreak" | "LineBreak") => out.push(' '),
            // Code and Math hold [attr-or-type, text].
            Some("Code" | "Math") => out.push_str(
                o.get("c")
                    .and_then(|c| c.get(1))
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            ),
            Some("RawInline" | "Note") => {}
            _ => {
                if let Some(c) = o.get("c") {
                    collect_text(c, out);
                }
            }
        },
        _ => {}
    }
}

struct Rewriter<'a> {
    ctx: &'a MediaContext<'a>,
    /// `extract_dir` with `/` separators and a trailing `/`.
    extract_prefix: String,
    namer: PictureNamer,
    /// Source picture file → assigned file name.
    copied: HashMap<PathBuf, String>,
    report: MediaReport,
}

impl Rewriter<'_> {
    fn walk(&mut self, v: &mut Value) -> io::Result<()> {
        match v {
            Value::Array(items) => items.iter_mut().try_for_each(|i| self.walk(i)),
            Value::Object(o) => {
                if o.get("t").and_then(Value::as_str) == Some("Image") {
                    if let Some(c) = o.get_mut("c") {
                        self.image(c)?;
                    }
                    Ok(())
                } else {
                    o.values_mut().try_for_each(|c| self.walk(c))
                }
            }
            _ => Ok(()),
        }
    }

    /// `c` is `[attr, [alt inlines], [src, title]]`.
    fn image(&mut self, c: &mut Value) -> io::Result<()> {
        let Some(src) = c
            .get(2)
            .and_then(|t| t.get(0))
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return Ok(());
        };
        let Some(file) = self.resolve(&src) else {
            return Ok(());
        };

        let key = file.canonicalize().unwrap_or_else(|_| file.clone());
        let name = match self.copied.get(&key) {
            Some(name) => name.clone(),
            None => {
                let alt = c.get(1).map(inline_text).unwrap_or_default();
                let title = c
                    .get(2)
                    .and_then(|t| t.get(1))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let ext = file
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let name = self.namer.name(&alt, title, self.report.pictures + 1, &ext);
                std::fs::create_dir_all(self.ctx.assets_dir)?;
                std::fs::copy(&file, self.ctx.assets_dir.join(&name))?;
                self.report.pictures += 1;
                self.copied.insert(key, name.clone());
                name
            }
        };
        c[2][0] = Value::String(format!("{}/{}", self.ctx.link_prefix, name));
        Ok(())
    }

    /// The picture file behind `src`, or `None` for remote links and for
    /// local ones that don't exist (which add a warning).
    fn resolve(&mut self, src: &str) -> Option<PathBuf> {
        let embedded = slashes(src).starts_with(&self.extract_prefix);
        if !embedded && is_remote(src) {
            return None;
        }
        let candidates = if embedded {
            vec![PathBuf::from(src), PathBuf::from(percent_decode(src))]
        } else {
            let decoded = PathBuf::from(percent_decode(src));
            let rel = PathBuf::from(src);
            [decoded, rel]
                .into_iter()
                .map(|p| {
                    if p.is_absolute() {
                        p
                    } else {
                        self.ctx.source_dir.join(p)
                    }
                })
                .collect()
        };
        let found = candidates.into_iter().find(|p| p.is_file());
        if found.is_none() {
            self.report
                .warnings
                .push(format!("Picture not found: {src}"));
        }
        found
    }
}

/// True for links with a URL scheme (`https:`, `data:`, `file:` …), which are
/// never fetched. A Windows drive path like `C:\x.png` is local.
fn is_remote(src: &str) -> bool {
    URL_SCHEME.is_match(src) && !WINDOWS_DRIVE.is_match(src)
}

fn slashes(s: &str) -> String {
    s.replace('\\', "/")
}

/// Decodes `%XX` escapes; returns the input unchanged if the result isn't UTF-8.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::path::PathBuf;
    use tempfile::TempDir;

    const ATTR: fn() -> Value = || json!(["", [], []]);

    fn img(alt: &str, src: &str, title: &str) -> Value {
        json!({"t": "Image", "c": [ATTR(), [{"t": "Str", "c": alt}], [src, title]]})
    }

    fn doc(inlines: Vec<Value>) -> Value {
        json!({"pandoc-api-version": [1, 23, 1, 2], "meta": {}, "blocks": [{"t": "Para", "c": inlines}]})
    }

    /// Every Image `src` in document order.
    fn srcs(v: &Value) -> Vec<String> {
        let mut out = Vec::new();
        fn walk(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::Object(o) => {
                    if o.get("t").and_then(Value::as_str) == Some("Image") {
                        out.push(o["c"][2][0].as_str().unwrap().to_string());
                    }
                    o.values().for_each(|c| walk(c, out));
                }
                Value::Array(a) => a.iter().for_each(|c| walk(c, out)),
                _ => {}
            }
        }
        walk(v, &mut out);
        out
    }

    struct Fx {
        _d: TempDir,
        extract: PathBuf,
        src: PathBuf,
        assets: PathBuf,
    }

    impl Fx {
        fn new() -> Fx {
            let d = tempfile::tempdir().unwrap();
            let (extract, src) = (d.path().join("extract"), d.path().join("src"));
            std::fs::create_dir_all(extract.join("media")).unwrap();
            std::fs::create_dir_all(&src).unwrap();
            std::fs::create_dir_all(d.path().join("out")).unwrap();
            let assets = d.path().join("out/report-assets");
            Fx {
                _d: d,
                extract,
                src,
                assets,
            }
        }
        fn put(&self, rel: &str, bytes: &[u8]) -> String {
            let p = self.extract.join(rel);
            std::fs::write(&p, bytes).unwrap();
            p.to_string_lossy().replace('\\', "/")
        }
        fn ctx(&self) -> MediaContext<'_> {
            MediaContext {
                extract_dir: &self.extract,
                source_dir: &self.src,
                assets_dir: &self.assets,
                link_prefix: "report-assets",
            }
        }
    }

    #[test]
    fn embedded_renamed_from_alt_and_sequence() {
        let fx = Fx::new();
        let a = fx.put("media/rId10.png", b"AAA");
        let b = fx.put("media/rId13.png", b"BBB");
        let mut ast = doc(vec![img("Sales Chart Q3", &a, ""), img("", &b, "")]);
        let r = rewrite_images(&mut ast, &fx.ctx()).unwrap();
        assert_eq!(r.pictures, 2);
        assert!(r.warnings.is_empty());
        assert_eq!(
            srcs(&ast),
            [
                "report-assets/sales-chart-q3.png",
                "report-assets/image-002.png"
            ]
        );
        assert_eq!(
            std::fs::read(fx.assets.join("sales-chart-q3.png")).unwrap(),
            b"AAA"
        );
        assert_eq!(
            std::fs::read(fx.assets.join("image-002.png")).unwrap(),
            b"BBB"
        );
    }

    #[test]
    fn same_file_twice_copied_once() {
        let fx = Fx::new();
        let a = fx.put("media/rId10.png", b"AAA");
        let mut ast = doc(vec![img("Logo", &a, ""), img("Other name", &a, "")]);
        let r = rewrite_images(&mut ast, &fx.ctx()).unwrap();
        assert_eq!(r.pictures, 1);
        assert_eq!(
            srcs(&ast),
            ["report-assets/logo.png", "report-assets/logo.png"]
        );
        assert_eq!(std::fs::read_dir(&fx.assets).unwrap().count(), 1);
    }

    #[test]
    fn remote_left_untouched_and_no_assets_dir() {
        let fx = Fx::new();
        let mut ast = doc(vec![img("x", "https://example.com/a.png", "")]);
        let r = rewrite_images(&mut ast, &fx.ctx()).unwrap();
        assert_eq!(r.pictures, 0);
        assert_eq!(srcs(&ast), ["https://example.com/a.png"]);
        assert!(!fx.assets.exists());
    }

    #[test]
    fn local_relative_copied_missing_warned() {
        let fx = Fx::new();
        std::fs::write(fx.src.join("local.png"), b"LOC").unwrap();
        let mut ast = doc(vec![
            img("Local pic", "local.png", ""),
            img("Missing pic", "missing.png", ""),
        ]);
        let r = rewrite_images(&mut ast, &fx.ctx()).unwrap();
        assert_eq!(r.pictures, 1);
        assert_eq!(srcs(&ast), ["report-assets/local-pic.png", "missing.png"]);
        assert_eq!(r.warnings, ["Picture not found: missing.png"]);
        assert_eq!(
            std::fs::read(fx.assets.join("local-pic.png")).unwrap(),
            b"LOC"
        );
    }

    #[test]
    fn nested_inlines_found_in_tables_and_figures() {
        let fx = Fx::new();
        let a = fx.put("media/a.png", b"A");
        let b = fx.put("media/b.png", b"B");
        let cell = |blocks: Value| json!([ATTR(), {"t": "AlignDefault"}, 1, 1, blocks]);
        let mut ast = json!({"pandoc-api-version": [1, 23, 1, 2], "meta": {}, "blocks": [
            {"t": "Figure", "c": [ATTR(), [null, [{"t": "Plain", "c": [{"t": "Str", "c": "Cap"}]}]],
                [{"t": "Plain", "c": [img("In figure", &a, "")]}]]},
            {"t": "Table", "c": [ATTR(), [null, []], [[{"t": "AlignDefault"}, {"t": "ColWidthDefault"}]],
                [ATTR(), []],
                [[ATTR(), 0, [], [[ATTR(), [cell(json!([{"t": "Plain", "c": [img("In table", &b, "")]}]))]]]]],
                [ATTR(), []]]}
        ]});
        let r = rewrite_images(&mut ast, &fx.ctx()).unwrap();
        assert_eq!(r.pictures, 2);
        assert_eq!(
            srcs(&ast),
            ["report-assets/in-figure.png", "report-assets/in-table.png"]
        );
    }

    #[test]
    fn inline_text_joins_strs_and_spaces() {
        let inlines = json!([{"t": "Str", "c": "Sales"}, {"t": "Space"}, {"t": "Strong", "c": [{"t": "Str", "c": "Chart"}]}]);
        assert_eq!(inline_text(&inlines), "Sales Chart");
    }

    #[test]
    fn remote_vs_local_classification() {
        for s in [
            "https://x/a.png",
            "http://x/a.png",
            "data:image/png;base64,AA",
            "file:///tmp/a.png",
        ] {
            assert!(is_remote(s), "{s}");
        }
        for s in [
            "a.png",
            "pics/a b.png",
            "/abs/a.png",
            "C:\\pics\\a.png",
            "C:/pics/a.png",
        ] {
            assert!(!is_remote(s), "{s}");
        }
    }
}
