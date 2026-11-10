//! Supported input formats and the pandoc reader for each.

use std::path::Path;

/// File extensions the app accepts, lowercase and without the dot.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "docx", "pptx", "xlsx", "odt", "fodt", "rtf", "epub", "html", "htm",
];

/// A source document format, chosen by file extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputFormat {
    Docx,
    Pptx,
    Xlsx,
    Odt,
    Fodt,
    Rtf,
    Epub,
    Html,
}

impl InputFormat {
    /// Detects the format from the extension (case-insensitive). Returns `None`
    /// for unsupported extensions and for names without a stem, such as `.docx`.
    pub fn from_path(path: &Path) -> Option<Self> {
        if path.file_stem().is_none_or(|s| s.is_empty()) {
            return None;
        }
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "docx" => Self::Docx,
            "pptx" => Self::Pptx,
            "xlsx" => Self::Xlsx,
            "odt" => Self::Odt,
            "fodt" => Self::Fodt,
            "rtf" => Self::Rtf,
            "epub" => Self::Epub,
            "html" | "htm" => Self::Html,
            _ => return None,
        })
    }

    /// The pandoc reader name passed with `-f`.
    pub fn reader(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Pptx => "pptx",
            Self::Xlsx => "xlsx",
            Self::Odt => "odt",
            Self::Fodt => "fodt",
            Self::Rtf => "rtf",
            Self::Epub => "epub",
            Self::Html => "html",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_extension_to_reader() {
        let cases = [
            ("a.docx", "docx"),
            ("a.pptx", "pptx"),
            ("a.xlsx", "xlsx"),
            ("a.odt", "odt"),
            ("a.fodt", "fodt"),
            ("a.rtf", "rtf"),
            ("a.epub", "epub"),
            ("a.html", "html"),
            ("a.htm", "html"),
        ];
        for (f, r) in cases {
            assert_eq!(
                InputFormat::from_path(Path::new(f)).unwrap().reader(),
                r,
                "{f}"
            );
        }
    }

    #[test]
    fn extension_is_case_insensitive() {
        assert_eq!(
            InputFormat::from_path(Path::new("X.DOCX")),
            Some(InputFormat::Docx)
        );
    }

    #[test]
    fn rejects_legacy_and_unknown() {
        for f in [
            "a.doc", "a.xls", "a.ppt", "a.pdf", "a.txt", "noext", ".docx",
        ] {
            assert_eq!(InputFormat::from_path(Path::new(f)), None, "{f}");
        }
    }
}
