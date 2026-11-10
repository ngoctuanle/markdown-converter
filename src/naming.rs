//! Output-name reservation: `S-converted.md` + `S-assets`, then `(n)` variants.

use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};

/// Highest index tried before giving up.
const MAX_INDEX: u32 = 10_000;

/// Returns `(md_file_name, assets_dir_name)` for `stem` at `index`.
/// Index 0 has no number; index n ≥ 1 adds ` (n)` / `-(n)`.
pub fn output_names(stem: &str, index: u32) -> (String, String) {
    if index == 0 {
        (format!("{stem}-converted.md"), format!("{stem}-assets"))
    } else {
        (
            format!("{stem}-converted ({index}).md"),
            format!("{stem}-assets-({index})"),
        )
    }
}

/// A claimed output slot. The `.md` file already exists (empty) on disk; the
/// assets folder does not exist yet and is created by whoever writes pictures.
#[derive(Debug)]
pub struct Reservation {
    pub md_path: PathBuf,
    pub assets_dir: PathBuf,
    pub index: u32,
    file: File,
}

/// Claims the lowest index whose `.md` and assets folder both don't exist.
///
/// Creating the `.md` with `create_new` is the lock, so concurrent callers
/// (other threads or processes) never get the same index.
pub fn reserve_output(out_dir: &Path, stem: &str) -> io::Result<Reservation> {
    for index in 0..=MAX_INDEX {
        let (md_name, assets_name) = output_names(stem, index);
        let assets_dir = out_dir.join(assets_name);
        if assets_dir.exists() {
            continue;
        }
        let md_path = out_dir.join(md_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&md_path)
        {
            Ok(file) => {
                return Ok(Reservation {
                    md_path,
                    assets_dir,
                    index,
                    file,
                });
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other(format!(
        "no free output name for \"{stem}\" after {MAX_INDEX} tries"
    )))
}

impl Reservation {
    /// The file-name component of `assets_dir`, used as the link prefix.
    pub fn assets_dir_name(&self) -> String {
        self.assets_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Writes the Markdown into the reserved file.
    pub fn write_markdown(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.write_all(bytes)?;
        self.file.flush()
    }

    /// Deletes the reserved `.md` and the assets folder (recursively), if present.
    pub fn discard(self) -> io::Result<()> {
        let Reservation {
            md_path,
            assets_dir,
            file,
            ..
        } = self;
        drop(file); // Windows can't delete a file that's still open.
        ignore_not_found(std::fs::remove_file(&md_path))?;
        ignore_not_found(std::fs::remove_dir_all(&assets_dir))
    }
}

fn ignore_not_found(r: io::Result<()>) -> io::Result<()> {
    match r {
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn names_for_index_zero_and_n() {
        assert_eq!(
            output_names("report", 0),
            ("report-converted.md".into(), "report-assets".into())
        );
        assert_eq!(
            output_names("report", 1),
            ("report-converted (1).md".into(), "report-assets-(1)".into())
        );
        assert_eq!(
            output_names("Q3 (final)", 2),
            (
                "Q3 (final)-converted (2).md".into(),
                "Q3 (final)-assets-(2)".into()
            )
        );
    }

    #[test]
    fn reserve_first_free_creates_empty_md_only() {
        let d = tempdir().unwrap();
        let r = reserve_output(d.path(), "report").unwrap();
        assert_eq!(r.index, 0);
        assert!(r.md_path.exists());
        assert!(!r.assets_dir.exists());
    }

    #[test]
    fn reserve_skips_when_md_exists() {
        let d = tempdir().unwrap();
        std::fs::write(d.path().join("report-converted.md"), "x").unwrap();
        assert_eq!(reserve_output(d.path(), "report").unwrap().index, 1);
    }

    #[test]
    fn reserve_skips_when_only_assets_exists() {
        let d = tempdir().unwrap();
        std::fs::create_dir(d.path().join("report-assets")).unwrap();
        std::fs::write(d.path().join("report-converted (1).md"), "x").unwrap();
        assert_eq!(reserve_output(d.path(), "report").unwrap().index, 2);
    }

    #[test]
    fn reserve_concurrent_same_stem_unique() {
        let d = tempdir().unwrap();
        let p = d.path().to_path_buf();
        let hs: Vec<_> = (0..16)
            .map(|_| {
                let p = p.clone();
                std::thread::spawn(move || reserve_output(&p, "report").unwrap().index)
            })
            .collect();
        let mut idx: Vec<u32> = hs.into_iter().map(|h| h.join().unwrap()).collect();
        idx.sort();
        assert_eq!(idx, (0..16).collect::<Vec<_>>());
    }

    #[test]
    fn discard_removes_md_and_assets() {
        let d = tempdir().unwrap();
        let r = reserve_output(d.path(), "a").unwrap();
        std::fs::create_dir(&r.assets_dir).unwrap();
        std::fs::write(r.assets_dir.join("x.png"), "x").unwrap();
        let (md, assets) = (r.md_path.clone(), r.assets_dir.clone());
        r.discard().unwrap();
        assert!(!md.exists() && !assets.exists());
    }

    #[cfg(unix)]
    #[test]
    fn reserve_readonly_dir_errors() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempdir().unwrap();
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        assert!(reserve_output(d.path(), "a").is_err());
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
