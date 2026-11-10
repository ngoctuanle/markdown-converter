//! Text helpers for the formatted preview. Nothing here is written to disk.

use regex::{Captures, Regex};
use std::sync::LazyLock;

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
            let target = caps.get(1).or_else(|| caps.get(2)).map_or("", |m| m.as_str());
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
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
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
        let md = "a ![Logo](r-assets/logo.emf) b ![x](r-assets/x.png) c ![](<My r-assets/y.WMF> \"t\")";
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
}
