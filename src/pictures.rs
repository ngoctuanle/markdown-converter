//! Picture file names: slug of alt text or title, else `image-NNN`, deduplicated.

use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

/// Longest slug, in bytes (slugs are ASCII).
const MAX_SLUG_LEN: usize = 60;

/// Extensions stripped from alt text such as `red.png`.
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "emf", "wmf", "svg", "webp",
];

/// Shape names Office generates on its own, such as "Picture 1".
static GENERIC_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(picture|image|img|graphic|figure|object|chart|photo)\s*[-_]?\s*\d*$")
        .unwrap()
});

/// Device names Windows won't accept as a file name, whatever the extension.
static WINDOWS_RESERVED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(con|prn|aux|nul|com[0-9]|lpt[0-9])$").unwrap());

/// Lowercase ASCII slug: transliterated, runs of other characters become `-`,
/// at most 60 bytes, cut at a `-` when one is available.
pub fn slugify(text: &str) -> String {
    let ascii = deunicode::deunicode(text).to_ascii_lowercase();
    let mut slug = String::with_capacity(ascii.len());
    for c in ascii.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let mut slug = slug.trim_matches('-');
    if slug.len() > MAX_SLUG_LEN {
        slug = &slug[..MAX_SLUG_LEN];
        if let Some(cut) = slug.rfind('-').filter(|&i| i > 0) {
            slug = &slug[..cut];
        }
        slug = slug.trim_matches('-');
    }
    slug.to_string()
}

/// True for names like "Picture 1", "image2", "IMG_3" or "Figure".
pub fn is_generic_name(text: &str) -> bool {
    GENERIC_NAME.is_match(text.trim())
}

/// The slug to name a picture by: from `alt`, else `title`. Skips empty and
/// generic names, and strips a trailing image extension (`red.png` → `red`).
pub fn candidate_name(alt: &str, title: &str) -> Option<String> {
    [alt, title].into_iter().find_map(|text| {
        let text = strip_image_extension(text.trim()).trim();
        if text.is_empty() || is_generic_name(text) {
            return None;
        }
        let slug = slugify(text);
        (!slug.is_empty() && !WINDOWS_RESERVED.is_match(&slug)).then_some(slug)
    })
}

fn strip_image_extension(text: &str) -> &str {
    match text.rsplit_once('.') {
        Some((stem, ext)) if IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) => stem,
        _ => text,
    }
}

/// Hands out unique picture file names within one document.
#[derive(Debug, Default)]
pub struct PictureNamer {
    used: HashSet<String>,
}

impl PictureNamer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Name for the picture at 1-based position `seq` with extension `ext`.
    /// Uniqueness is by base name, so `logo.png` and `logo.emf` don't both appear.
    pub fn name(&mut self, alt: &str, title: &str, seq: usize, ext: &str) -> String {
        let base = candidate_name(alt, title).unwrap_or_else(|| format!("image-{seq:03}"));
        let mut unique = base.clone();
        let mut n = 2;
        while !self.used.insert(unique.clone()) {
            unique = format!("{base}-{n}");
            n += 1;
        }
        if ext.is_empty() {
            unique
        } else {
            format!("{unique}.{}", ext.to_ascii_lowercase())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic_and_unicode() {
        assert_eq!(slugify("Sales Chart Q3"), "sales-chart-q3");
        assert_eq!(slugify("Biểu đồ doanh thu"), "bieu-do-doanh-thu");
        assert_eq!(slugify("  --A/B__c  "), "a-b-c");
        assert!(slugify(&"word ".repeat(40)).len() <= 60);
    }

    #[test]
    fn generic_titles_ignored() {
        for t in ["Picture 1", "image2", "Graphic 12", "Figure", "IMG_3"] {
            assert!(is_generic_name(t), "{t}");
        }
        assert!(!is_generic_name("Picture of the team"));
        assert_eq!(candidate_name("", "Picture 1"), None);
    }

    #[test]
    fn candidate_prefers_alt_then_title() {
        assert_eq!(
            candidate_name("Sales Chart Q3", "Picture 1").as_deref(),
            Some("sales-chart-q3")
        );
        assert_eq!(
            candidate_name("", "Team photo").as_deref(),
            Some("team-photo")
        );
        assert_eq!(candidate_name("   ", "  "), None);
    }

    #[test]
    fn candidate_strips_image_extension() {
        assert_eq!(
            candidate_name("red.png", "Picture 1").as_deref(),
            Some("red")
        );
    }

    #[test]
    fn namer_sequence_fallback_and_dedup() {
        let mut n = PictureNamer::new();
        assert_eq!(n.name("Logo", "", 1, "PNG"), "logo.png");
        assert_eq!(n.name("Logo", "", 2, "png"), "logo-2.png");
        assert_eq!(n.name("", "", 3, "emf"), "image-003.emf");
        assert_eq!(n.name("", "", 4, ""), "image-004");
    }

    #[test]
    fn namer_never_repeats_a_generated_name() {
        let mut n = PictureNamer::new();
        assert_eq!(n.name("Logo", "", 1, "png"), "logo.png");
        assert_eq!(n.name("Logo", "", 2, "png"), "logo-2.png");
        assert_eq!(n.name("Logo 2", "", 3, "png"), "logo-2-2.png");
    }

    #[test]
    fn windows_reserved_names_fall_back() {
        for t in ["CON", "nul", "Aux", "com1", "LPT9", "prn.png"] {
            assert_eq!(candidate_name(t, ""), None, "{t}");
        }
        assert_eq!(candidate_name("console", "").as_deref(), Some("console"));
    }
}
