//! Copy, rename and relink the pictures in a pandoc JSON AST.

use crate::pictures::PictureNamer;
use base64::Engine;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
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

/// Rewrites every Image in `ast`, in document order: embedded pictures, local
/// picture files and `data:` pictures are saved into `assets_dir` under
/// readable names and relinked; remote ones are left alone; local links that
/// are missing, not a picture file or on a network share are left alone with
/// a warning.
pub fn rewrite_images(ast: &mut Value, ctx: &MediaContext) -> io::Result<MediaReport> {
    let mut rw = Rewriter {
        ctx,
        extract_prefix: format!(
            "{}/",
            slashes(&ctx.extract_dir.to_string_lossy()).trim_end_matches('/')
        ),
        namer: PictureNamer::new(),
        copied: HashMap::new(),
        saved: HashMap::new(),
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

/// Extensions a linked local file must have to be copied (case-insensitive).
const PICTURE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "emf", "wmf", "svg", "webp",
];

struct Rewriter<'a> {
    ctx: &'a MediaContext<'a>,
    /// `extract_dir` with `/` separators and a trailing `/`.
    extract_prefix: String,
    namer: PictureNamer,
    /// Source picture file → assigned file name.
    copied: HashMap<PathBuf, String>,
    /// Decoded `data:` picture bytes → assigned file name.
    saved: HashMap<Vec<u8>, String>,
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
        let name = if is_data_uri(&src) {
            let Some((bytes, ext)) = self.decode_data_uri(&src) else {
                return Ok(());
            };
            match self.saved.get(&bytes) {
                Some(name) => name.clone(),
                None => {
                    let name = self.new_name(c, ext);
                    std::fs::create_dir_all(self.ctx.assets_dir)?;
                    std::fs::write(self.ctx.assets_dir.join(&name), &bytes)?;
                    self.report.pictures += 1;
                    self.saved.insert(bytes, name.clone());
                    name
                }
            }
        } else {
            let Some(file) = self.resolve(&src) else {
                return Ok(());
            };
            let key = file.canonicalize().unwrap_or_else(|_| file.clone());
            match self.copied.get(&key) {
                Some(name) => name.clone(),
                None => {
                    let ext = file
                        .extension()
                        .map(|e| e.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let name = self.new_name(c, &ext);
                    std::fs::create_dir_all(self.ctx.assets_dir)?;
                    std::fs::copy(&file, self.ctx.assets_dir.join(&name))?;
                    self.report.pictures += 1;
                    self.copied.insert(key, name.clone());
                    name
                }
            }
        };
        c[2][0] = Value::String(escape_link(&format!("{}/{}", self.ctx.link_prefix, name)));
        Ok(())
    }

    /// A fresh file name for the picture in image `c`, from its alt text or title.
    fn new_name(&mut self, c: &Value, ext: &str) -> String {
        let alt = c.get(1).map(inline_text).unwrap_or_default();
        let title = c
            .get(2)
            .and_then(|t| t.get(1))
            .and_then(Value::as_str)
            .unwrap_or("");
        self.namer.name(&alt, title, self.report.pictures + 1, ext)
    }

    /// The bytes and file extension of a `data:image/<type>;base64,…` link, or
    /// `None` (with a warning) for any other kind of `data:` link.
    fn decode_data_uri(&mut self, src: &str) -> Option<(Vec<u8>, &'static str)> {
        let decoded = src.split_once(',').and_then(|(header, payload)| {
            let header = header.to_ascii_lowercase();
            let mime = header
                .strip_suffix(";base64")?
                .strip_prefix("data:image/")?;
            let subtype = mime.split(';').next().unwrap_or("");
            let ext = data_picture_extension(subtype)?;
            let payload: String = payload
                .chars()
                .filter(|c| !c.is_ascii_whitespace())
                .collect();
            let bytes = BASE64.decode(payload).ok()?;
            Some((bytes, ext))
        });
        if decoded.is_none() {
            let header = src.split(',').next().unwrap_or(src);
            self.report
                .warnings
                .push(format!("Picture not copied: {header},…"));
        }
        decoded
    }

    /// The picture file behind `src`, or `None` for remote links, for local
    /// ones that may not be copied, and for missing ones (both with a warning).
    fn resolve(&mut self, src: &str) -> Option<PathBuf> {
        let embedded = slashes(src).starts_with(&self.extract_prefix);
        let file_url = if embedded { None } else { file_url_path(src) };
        if !embedded && file_url.is_none() && is_remote(src) {
            return None;
        }
        let candidates = if embedded {
            vec![PathBuf::from(src), PathBuf::from(percent_decode(src))]
        } else {
            let local = match &file_url {
                Some(path) => vec![path.clone()],
                None => vec![percent_decode(src), src.to_string()],
            };
            // Never touch network paths, and copy only picture files.
            if local.iter().any(|p| is_network_path(p))
                || !local.iter().any(|p| has_picture_extension(p))
            {
                self.report
                    .warnings
                    .push(format!("Picture not copied: {src}"));
                return None;
            }
            local
                .into_iter()
                .filter(|p| has_picture_extension(p))
                .map(PathBuf::from)
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

static BASE64: LazyLock<GeneralPurpose> = LazyLock::new(|| {
    GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    )
});

fn is_data_uri(src: &str) -> bool {
    src.get(..5)
        .is_some_and(|s| s.eq_ignore_ascii_case("data:"))
}

/// File extension for a `data:image/<subtype>` picture we know how to save.
fn data_picture_extension(subtype: &str) -> Option<&'static str> {
    Some(match subtype {
        "png" => "png",
        "jpeg" | "jpg" => "jpg",
        "gif" => "gif",
        "webp" => "webp",
        "svg+xml" => "svg",
        "bmp" => "bmp",
        "tiff" => "tif",
        _ => return None,
    })
}

/// The local path in a `file:` URL, percent-decoded. A URL naming another
/// host comes back as a `//host/…` network path (which is then refused).
fn file_url_path(src: &str) -> Option<String> {
    let rest = src
        .get(..5)
        .filter(|s| s.eq_ignore_ascii_case("file:"))
        .map(|_| &src[5..])?;
    let path = match rest.strip_prefix("//") {
        Some(after) => {
            let (host, path) = after.split_at(after.find('/').unwrap_or(after.len()));
            if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
                path.to_string()
            } else {
                format!("//{after}")
            }
        }
        None => rest.to_string(),
    };
    let path = percent_decode(&path);
    // `/C:/pics/a.png` → `C:/pics/a.png`
    Some(match path.strip_prefix('/') {
        Some(drive) if WINDOWS_DRIVE.is_match(drive) => drive.to_string(),
        _ => path,
    })
}

/// True for UNC and other network paths (`\\host\share`, `//host/share`),
/// which would make Windows contact another machine.
fn is_network_path(path: &str) -> bool {
    let path = slashes(path);
    path.starts_with("//")
}

fn has_picture_extension(path: &str) -> bool {
    Path::new(&slashes(path))
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| PICTURE_EXTENSIONS.iter().any(|p| e.eq_ignore_ascii_case(p)))
}

/// Escapes the characters that would end or break a link target: `%`, `#`, `?`.
fn escape_link(link: &str) -> String {
    let mut out = String::with_capacity(link.len());
    for ch in link.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            _ => out.push(ch),
        }
    }
    out
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

    fn assert_not_copied(src: &str) {
        let fx = Fx::new();
        let mut ast = doc(vec![img("x", src, "")]);
        let r = rewrite_images(&mut ast, &fx.ctx()).unwrap();
        assert_eq!(r.pictures, 0, "{src}");
        assert_eq!(srcs(&ast), [src]);
        assert_eq!(r.warnings, [format!("Picture not copied: {src}")]);
        assert!(!fx.assets.exists());
    }

    #[test]
    fn unc_and_network_paths_rejected() {
        for src in [
            r"\\host\share\x.png",
            "//host/share/x.png",
            "file://host/share/x.png",
        ] {
            assert_not_copied(src);
        }
    }

    #[test]
    fn local_non_picture_files_rejected() {
        let fx = Fx::new();
        std::fs::write(fx.src.join("notes.txt"), b"secret").unwrap();
        std::fs::write(fx.src.join("id_rsa"), b"key").unwrap();
        let abs = fx.src.join("id_rsa").to_string_lossy().into_owned();
        for src in ["notes.txt", "id_rsa", abs.as_str()] {
            let mut ast = doc(vec![img("x", src, "")]);
            let r = rewrite_images(&mut ast, &fx.ctx()).unwrap();
            assert_eq!(r.pictures, 0, "{src}");
            assert_eq!(srcs(&ast), [src]);
            assert_eq!(r.warnings, [format!("Picture not copied: {src}")]);
        }
        assert!(!fx.assets.exists());
    }

    #[test]
    fn picture_extension_case_insensitive_and_parent_dirs_allowed() {
        let fx = Fx::new();
        let images = fx.src.parent().unwrap().join("images");
        std::fs::create_dir_all(&images).unwrap();
        std::fs::write(images.join("up.png"), b"UP").unwrap();
        std::fs::write(fx.src.join("BIG.JPEG"), b"BIG").unwrap();
        let mut ast = doc(vec![
            img("Up", "../images/up.png", ""),
            img("Big", "BIG.JPEG", ""),
        ]);
        let r = rewrite_images(&mut ast, &fx.ctx()).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(r.pictures, 2);
        assert_eq!(
            srcs(&ast),
            ["report-assets/up.png", "report-assets/big.jpeg"]
        );
    }

    #[test]
    fn file_urls_are_local_paths() {
        let fx = Fx::new();
        std::fs::write(fx.src.join("my pic.png"), b"PIC").unwrap();
        let abs = slashes(&fx.src.join("my pic.png").to_string_lossy()).replace(' ', "%20");
        let abs = abs.trim_start_matches('/');
        let mut ast = doc(vec![
            img("A", &format!("file:///{abs}"), ""),
            img("B", &format!("file://localhost/{abs}"), ""),
            img("C", "file:///nowhere/gone.png", ""),
        ]);
        let r = rewrite_images(&mut ast, &fx.ctx()).unwrap();
        assert_eq!(r.pictures, 1);
        assert_eq!(
            srcs(&ast),
            [
                "report-assets/a.png",
                "report-assets/a.png",
                "file:///nowhere/gone.png"
            ]
        );
        assert_eq!(r.warnings, ["Picture not found: file:///nowhere/gone.png"]);
    }

    #[test]
    fn file_url_to_path_forms() {
        assert_eq!(
            file_url_path("file:///tmp/a%20b.png").as_deref(),
            Some("/tmp/a b.png")
        );
        assert_eq!(
            file_url_path("FILE://localhost/tmp/a.png").as_deref(),
            Some("/tmp/a.png")
        );
        assert_eq!(
            file_url_path("file:///C:/pics/a.png").as_deref(),
            Some("C:/pics/a.png")
        );
        assert_eq!(
            file_url_path("file://host/share/a.png").as_deref(),
            Some("//host/share/a.png")
        );
        assert_eq!(file_url_path("https://x/a.png"), None);
    }

    // A 1x1 PNG and a different payload.
    const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

    #[test]
    fn data_uris_saved_as_pictures_once_per_payload() {
        let fx = Fx::new();
        let png = format!("data:image/png;base64,{PNG_B64}");
        let jpeg = "data:image/jpeg;base64,/9j/4AAQ";
        let svg = "data:image/svg+xml;base64,PHN2Zy8+";
        let mut ast = doc(vec![
            img("Logo", &png, ""),
            img("Again", &png, ""),
            img("", jpeg, ""),
            img("", svg, ""),
        ]);
        let r = rewrite_images(&mut ast, &fx.ctx()).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(r.pictures, 3);
        assert_eq!(
            srcs(&ast),
            [
                "report-assets/logo.png",
                "report-assets/logo.png",
                "report-assets/image-002.jpg",
                "report-assets/image-003.svg"
            ]
        );
        assert_eq!(
            std::fs::read(fx.assets.join("image-003.svg")).unwrap(),
            b"<svg/>"
        );
        assert_eq!(std::fs::read_dir(&fx.assets).unwrap().count(), 3);
    }

    #[test]
    fn data_uris_unknown_type_or_bad_base64_left_with_warning() {
        let fx = Fx::new();
        let unknown = "data:image/x-icon;base64,AAAA";
        let bad = "data:image/png;base64,***not base64***";
        let plain = "data:image/svg+xml;utf8,<svg/>";
        let mut ast = doc(vec![
            img("", unknown, ""),
            img("", bad, ""),
            img("", plain, ""),
        ]);
        let r = rewrite_images(&mut ast, &fx.ctx()).unwrap();
        assert_eq!(r.pictures, 0);
        assert_eq!(srcs(&ast), [unknown, bad, plain]);
        assert_eq!(
            r.warnings,
            [
                "Picture not copied: data:image/x-icon;base64,…",
                "Picture not copied: data:image/png;base64,…",
                "Picture not copied: data:image/svg+xml;utf8,…",
            ]
        );
        assert!(!fx.assets.exists());
    }

    #[test]
    fn links_escape_percent_hash_and_question_mark() {
        let fx = Fx::new();
        let a = fx.put("media/a.png", b"A");
        let mut ast = doc(vec![img("Pic", &a, "")]);
        let ctx = MediaContext {
            link_prefix: "C# 100%?-assets",
            ..fx.ctx()
        };
        rewrite_images(&mut ast, &ctx).unwrap();
        assert_eq!(srcs(&ast), ["C%23 100%25%3F-assets/pic.png"]);
        assert!(fx.assets.join("pic.png").is_file());
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
