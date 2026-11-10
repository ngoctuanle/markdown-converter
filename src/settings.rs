//! Output-folder preference, saved as JSON in the user's config folder.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Settings {
    pub fixed_output: bool,
    pub fixed_output_dir: Option<PathBuf>,
}

impl Settings {
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("markdown-converter").join("settings.json"))
    }

    /// Missing or unreadable settings fall back to the defaults.
    pub fn load_from(path: &Path) -> Settings {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Where the Markdown for `source` goes: the fixed folder if one is set, else next to the source.
    pub fn output_dir_for(&self, source: &Path) -> PathBuf {
        match (self.fixed_output, &self.fixed_output_dir) {
            (true, Some(dir)) => dir.clone(),
            _ => source.parent().map(Path::to_path_buf).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn output_dir_default_and_fixed() {
        let s = Settings::default();
        assert_eq!(s.output_dir_for(Path::new("/a/b/r.docx")), PathBuf::from("/a/b"));
        let f = Settings { fixed_output: true, fixed_output_dir: Some("/out".into()) };
        assert_eq!(f.output_dir_for(Path::new("/a/b/r.docx")), PathBuf::from("/out"));
        let unset = Settings { fixed_output: true, fixed_output_dir: None };
        assert_eq!(unset.output_dir_for(Path::new("/a/b/r.docx")), PathBuf::from("/a/b"));
    }

    #[test]
    fn save_load_roundtrip_and_corrupt_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/settings.json");
        let s = Settings { fixed_output: true, fixed_output_dir: Some("/out".into()) };
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);
        std::fs::write(&path, "{{garbage").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        assert_eq!(Settings::load_from(&dir.path().join("missing.json")), Settings::default());
    }
}
