//! Text helpers for the formatted preview. Nothing here is written to disk.

use regex::{Captures, Regex};
use std::io::{self, Read};
use std::path::Path;
use std::sync::LazyLock;

/// Most of a Markdown file the preview loads, in bytes.
pub const PREVIEW_LIMIT: usize = 2 * 1024 * 1024;

/// A Markdown file as read for the preview.
#[derive(Debug)]
pub struct PreviewText {
    /// The text as on disk (up to the limit).
    pub raw: String,
    /// `raw` with undrawable pictures swapped for placeholders.
    pub formatted: String,
    /// True if the file is longer than what was read.
    pub truncated: bool,
}

/// Reads at most `limit` bytes of `path` (cut at a line break, else at a
/// character boundary) and prepares it for both preview modes. Meant to run
/// off the UI thread. The file itself is not changed.
pub fn load_preview(path: &Path, limit: usize) -> io::Result<PreviewText> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    let (kept, truncated) = cut_at_limit(&bytes, limit);
    let raw = String::from_utf8(kept.to_vec())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let formatted = placeholder_unrenderable_images(&raw);
    Ok(PreviewText {
        raw,
        formatted,
        truncated,
    })
}

/// `bytes` cut to at most `limit` bytes, after the last line break if there is
/// one, else on a UTF-8 character boundary; and whether anything was cut.
fn cut_at_limit(bytes: &[u8], limit: usize) -> (&[u8], bool) {
    if bytes.len() <= limit {
        return (bytes, false);
    }
    let head = &bytes[..limit];
    let end = match head.iter().rposition(|&b| b == b'\n') {
        Some(newline) => newline + 1,
        None => {
            // Back off continuation bytes (10xxxxxx) to the start of a character.
            let mut end = limit;
            while end > 0 && bytes[end] & 0xC0 == 0x80 {
                end -= 1;
            }
            end
        }
    };
    (&bytes[..end], true)
}

/// A Markdown image: `![alt](target)` or `![alt](<target with spaces> "title")`.
static IMAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"!\[[^\]]*\]\(\s*(?:<([^>\n]*)>|([^)\s]+))(?:\s+"[^"\n]*")?\s*\)"#).unwrap()
});

/// Picture formats the preview can't draw.
const UNRENDERABLE: [&str; 6] = ["emf", "wmf", "tif", "tiff", "eps", "svgz"];

/// Replaces each image the preview can't draw with `` `[picture: <file name>]` ``,
/// leaving everything else untouched.
pub fn placeholder_unrenderable_images(md: &str) -> String {
    IMAGE
        .replace_all(md, |caps: &Captures| {
            let target = caps
                .get(1)
                .or_else(|| caps.get(2))
                .map_or("", |m| m.as_str());
            if is_unrenderable(target) {
                let name = target.rsplit('/').next().unwrap_or(target);
                format!("`[picture: {}]`", percent_decode(name))
            } else {
                caps[0].to_string()
            }
        })
        .into_owned()
}

fn is_unrenderable(target: &str) -> bool {
    target
        .rsplit_once('.')
        .is_some_and(|(_, ext)| UNRENDERABLE.iter().any(|u| ext.eq_ignore_ascii_case(u)))
}

/// True for a link like `report-assets/x.png`: no URL scheme and not an absolute path.
pub fn is_relative_path(uri: &str) -> bool {
    static SCHEME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9+.-]*:").unwrap());
    !SCHEME.is_match(uri) && !uri.starts_with('/') && !uri.starts_with('\\')
}

/// Decodes `%XX` escapes, e.g. `My%20pic.png` → `My pic.png`. Malformed escapes stay as they are.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok());
        match (bytes[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_unrenderable_keeps_others() {
        let md =
            "a ![Logo](r-assets/logo.emf) b ![x](r-assets/x.png) c ![](<My r-assets/y.WMF> \"t\")";
        assert_eq!(
            placeholder_unrenderable_images(md),
            "a `[picture: logo.emf]` b ![x](r-assets/x.png) c `[picture: y.WMF]`"
        );
    }

    #[test]
    fn no_images_unchanged() {
        assert_eq!(placeholder_unrenderable_images("# Hi"), "# Hi");
    }

    #[test]
    fn relative_paths_have_no_scheme_and_no_root() {
        assert!(is_relative_path("report-assets/x.png"));
        assert!(is_relative_path("Báo cáo Q3 (final)-assets/image-001.png"));
        assert!(!is_relative_path("https://example.com/x.png"));
        assert!(!is_relative_path("data:image/png;base64,AAAA"));
        assert!(!is_relative_path("file:///tmp/x.png"));
        assert!(!is_relative_path("/tmp/x.png"));
    }

    #[test]
    fn percent_decode_handles_utf8_and_bad_escapes() {
        assert_eq!(percent_decode("My%20pic%281%29.png"), "My pic(1).png");
        assert_eq!(percent_decode("B%C3%A1o.png"), "Báo.png");
        assert_eq!(percent_decode("100%.png"), "100%.png");
        assert_eq!(percent_decode("a%zz"), "a%zz");
    }

    #[test]
    fn cut_keeps_short_text_whole() {
        assert_eq!(
            cut_at_limit(b"# Hi\nthere", 100),
            (&b"# Hi\nthere"[..], false)
        );
        assert_eq!(cut_at_limit(b"abc", 3), (&b"abc"[..], false));
    }

    #[test]
    fn cut_long_text_at_last_line_break() {
        assert_eq!(
            cut_at_limit(b"one\ntwo\nthree", 9),
            (&b"one\ntwo\n"[..], true)
        );
    }

    #[test]
    fn cut_without_line_break_stays_on_char_boundary() {
        // "ééé" is 6 bytes; 5 would split the last é.
        let (kept, cut) = cut_at_limit("ééé".as_bytes(), 5);
        assert!(cut);
        assert_eq!(std::str::from_utf8(kept).unwrap(), "éé");
    }

    #[test]
    fn load_preview_reads_at_most_the_limit() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("a.md");
        std::fs::write(&path, "line ![x](a-assets/x.emf)\n".repeat(10)).unwrap();
        let whole = load_preview(&path, 1000).unwrap();
        assert!(!whole.truncated);
        assert_eq!(whole.raw.lines().count(), 10);
        assert!(whole.formatted.contains("`[picture: x.emf]`"));

        let part = load_preview(&path, 60).unwrap();
        assert!(part.truncated);
        assert_eq!(part.raw, "line ![x](a-assets/x.emf)\n".repeat(2));
        assert!(load_preview(&d.path().join("gone.md"), 10).is_err());
    }

    #[test]
    fn escaped_picture_links_resolve_for_the_preview() {
        let link = "C%23 notes 100%25%3F-assets/pic.png";
        assert!(is_relative_path(link));
        assert_eq!(percent_decode(link), "C# notes 100%?-assets/pic.png");
    }
}
