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
///
/// Dropping a reservation without calling [`Reservation::commit`] (on an
/// error, a panic, or a cancelled conversion) deletes the `.md` and the
/// assets folder, so a failed file leaves nothing behind.
#[derive(Debug)]
pub struct Reservation {
    pub md_path: PathBuf,
    pub assets_dir: PathBuf,
    pub index: u32,
    /// `None` once committed or dropped.
    file: Option<File>,
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
                    file: Some(file),
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
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("reservation already closed"))?;
        file.write_all(bytes)?;
        file.flush()
    }

    /// Keeps the output: the `.md` and assets folder are no longer deleted on drop.
    pub fn commit(mut self) {
        self.file = None;
    }
}

impl Drop for Reservation {
    /// Deletes the reserved `.md` and the assets folder (recursively) unless committed.
    fn drop(&mut self) {
        if self.file.take().is_some() {
            // The file is closed first: Windows can't delete a file that's still open.
            let _ = std::fs::remove_file(&self.md_path);
            let _ = std::fs::remove_dir_all(&self.assets_dir);
        }
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
                // Returned, not dropped: dropping would free the slot again.
                std::thread::spawn(move || reserve_output(&p, "report").unwrap())
            })
            .collect();
        let held: Vec<Reservation> = hs.into_iter().map(|h| h.join().unwrap()).collect();
        let mut idx: Vec<u32> = held.iter().map(|r| r.index).collect();
        idx.sort();
        assert_eq!(idx, (0..16).collect::<Vec<_>>());
    }

    #[test]
    fn dropped_without_commit_leaves_nothing() {
        let d = tempdir().unwrap();
        let mut r = reserve_output(d.path(), "a").unwrap();
        std::fs::create_dir(&r.assets_dir).unwrap();
        std::fs::write(r.assets_dir.join("x.png"), "x").unwrap();
        r.write_markdown(b"# partial").unwrap();
        drop(r);
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0);
    }

    #[test]
    fn committed_reservation_is_kept() {
        let d = tempdir().unwrap();
        let mut r = reserve_output(d.path(), "a").unwrap();
        std::fs::create_dir(&r.assets_dir).unwrap();
        r.write_markdown(b"# done").unwrap();
        let (md, assets) = (r.md_path.clone(), r.assets_dir.clone());
        r.commit();
        assert_eq!(std::fs::read_to_string(md).unwrap(), "# done");
        assert!(assets.is_dir());
    }

    #[test]
    fn panic_after_reservation_leaves_nothing() {
        let d = tempdir().unwrap();
        let p = d.path().to_path_buf();
        let result = std::panic::catch_unwind(move || {
            let r = reserve_output(&p, "a").unwrap();
            std::fs::create_dir(&r.assets_dir).unwrap();
            std::fs::write(r.assets_dir.join("x.png"), "x").unwrap();
            panic!("boom");
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0);
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
