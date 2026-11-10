//! The list of files shown in the window, and what state each one is in.

use crate::convert::{ConvertError, ConvertOutcome};
use crate::formats::InputFormat;
use crate::queue::{JobId, WorkerEvent};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub enum FileStatus {
    Waiting,
    Queued,
    Converting,
    Done(ConvertOutcome),
    Failed(ConvertError),
}

impl FileStatus {
    pub fn label(&self) -> &'static str {
        match self {
            FileStatus::Waiting | FileStatus::Queued => "Waiting",
            FileStatus::Converting => "Converting",
            FileStatus::Done(_) => "Done",
            FileStatus::Failed(_) => "Failed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct FileItem {
    pub id: JobId,
    pub source: PathBuf,
    pub status: FileStatus,
}

#[derive(Debug, Default)]
pub struct AddReport {
    pub added: Vec<JobId>,
    pub duplicates: usize,
    pub unsupported: Vec<PathBuf>,
}

#[derive(Default)]
pub struct FileList {
    items: Vec<FileItem>,
    /// Canonical path of each item (raw path if it can't be canonicalized), parallel to `items`.
    keys: Vec<PathBuf>,
    last_id: JobId,
}

fn key_of(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn is_supported(path: &Path) -> bool {
    InputFormat::from_path(path).is_some()
}

impl FileList {
    pub fn add_paths(&mut self, paths: &[PathBuf]) -> AddReport {
        let mut report = AddReport::default();
        for path in paths {
            if path.is_dir() {
                for child in supported_children(path) {
                    self.add_one(child, &mut report);
                }
            } else if is_supported(path) {
                self.add_one(path.clone(), &mut report);
            } else {
                report.unsupported.push(path.clone());
            }
        }
        report
    }

    fn add_one(&mut self, path: PathBuf, report: &mut AddReport) {
        let key = key_of(&path);
        if self.keys.contains(&key) {
            report.duplicates += 1;
            return;
        }
        self.last_id += 1;
        self.items.push(FileItem { id: self.last_id, source: path, status: FileStatus::Waiting });
        self.keys.push(key);
        report.added.push(self.last_id);
    }

    pub fn items(&self) -> &[FileItem] {
        &self.items
    }

    pub fn get(&self, id: JobId) -> Option<&FileItem> {
        self.items.iter().find(|i| i.id == id)
    }

    fn get_mut(&mut self, id: JobId) -> Option<&mut FileItem> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    /// Files a press of Convert would start: Waiting and Failed.
    pub fn convertible_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.status, FileStatus::Waiting | FileStatus::Failed(_)))
            .count()
    }

    pub fn is_batch_running(&self) -> bool {
        self.items
            .iter()
            .any(|i| matches!(i.status, FileStatus::Queued | FileStatus::Converting))
    }

    pub fn take_for_convert(&mut self) -> Vec<(JobId, PathBuf)> {
        let mut jobs = Vec::new();
        for item in &mut self.items {
            if matches!(item.status, FileStatus::Waiting | FileStatus::Failed(_)) {
                item.status = FileStatus::Queued;
                jobs.push((item.id, item.source.clone()));
            }
        }
        jobs
    }

    pub fn queue_ids(&mut self, ids: &[JobId]) -> Vec<(JobId, PathBuf)> {
        let wanted: HashSet<JobId> = ids.iter().copied().collect();
        let mut jobs = Vec::new();
        for item in &mut self.items {
            if wanted.contains(&item.id) && matches!(item.status, FileStatus::Waiting) {
                item.status = FileStatus::Queued;
                jobs.push((item.id, item.source.clone()));
            }
        }
        jobs
    }

    pub fn retry(&mut self, id: JobId) -> Option<(JobId, PathBuf)> {
        let item = self.get_mut(id)?;
        if !matches!(item.status, FileStatus::Failed(_)) {
            return None;
        }
        item.status = FileStatus::Queued;
        Some((item.id, item.source.clone()))
    }

    pub fn remove(&mut self, id: JobId) -> bool {
        let Some(pos) = self.items.iter().position(|i| i.id == id) else {
            return false;
        };
        if !matches!(self.items[pos].status, FileStatus::Waiting | FileStatus::Queued) {
            return false;
        }
        self.items.remove(pos);
        self.keys.remove(pos);
        true
    }

    pub fn clear_finished(&mut self) {
        let mut i = 0;
        while i < self.items.len() {
            if matches!(self.items[i].status, FileStatus::Done(_) | FileStatus::Failed(_)) {
                self.items.remove(i);
                self.keys.remove(i);
            } else {
                i += 1;
            }
        }
    }

    pub fn apply(&mut self, ev: WorkerEvent) {
        match ev {
            WorkerEvent::Started(id) => {
                if let Some(item) = self.get_mut(id) {
                    item.status = FileStatus::Converting;
                }
            }
            WorkerEvent::Finished(id, result) => {
                if let Some(item) = self.get_mut(id) {
                    item.status = match result {
                        Ok(outcome) => FileStatus::Done(outcome),
                        Err(err) => FileStatus::Failed(err),
                    };
                }
            }
        }
    }
}

/// Direct children of `dir` with a supported extension, sorted by name.
/// Office lock files (`~$x.docx`) and hidden files such as macOS `._x.docx` are skipped.
fn supported_children(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            !name.starts_with("~$") && !name.starts_with('.')
        })
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_supported(p))
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{ConvertError, ConvertOutcome};
    use crate::queue::WorkerEvent;
    use std::fs;
    use std::path::PathBuf;

    fn outcome() -> ConvertOutcome {
        ConvertOutcome {
            md_path: PathBuf::from("/o/a.md"),
            assets_dir: None,
            pictures: 0,
            warnings: vec![],
        }
    }

    fn error() -> ConvertError {
        ConvertError { reason: "boom".into(), detail: String::new() }
    }

    fn list_with(names: &[&str]) -> FileList {
        let mut l = FileList::default();
        let paths: Vec<PathBuf> = names.iter().map(|n| PathBuf::from(format!("/nowhere/{n}"))).collect();
        l.add_paths(&paths);
        l
    }

    fn labels(l: &FileList) -> Vec<&'static str> {
        l.items().iter().map(|i| i.status.label()).collect()
    }

    #[test]
    fn add_files_folders_duplicates_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for f in ["a.docx", "b.pptx", "notes.txt"] {
            fs::write(d.join(f), b"x").unwrap();
        }
        fs::create_dir(d.join("sub")).unwrap();
        fs::write(d.join("sub/c.docx"), b"x").unwrap();

        let mut l = FileList::default();
        let report = l.add_paths(&[d.join("a.docx"), d.to_path_buf(), d.join("a.docx"), d.join("x.doc")]);

        assert_eq!(report.added, vec![1, 2]);
        assert_eq!(l.get(1).unwrap().source, d.join("a.docx"));
        assert_eq!(l.get(2).unwrap().source, d.join("b.pptx"));
        assert_eq!(report.duplicates, 2);
        assert_eq!(report.unsupported, vec![d.join("x.doc")]);
        assert_eq!(l.items().len(), 2);
    }

    #[test]
    fn folder_skips_lock_and_hidden_files_but_explicit_files_added() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for f in ["a.docx", "~$a.docx", "._a.docx", ".hidden.docx"] {
            fs::write(d.join(f), b"x").unwrap();
        }
        let mut l = FileList::default();
        let report = l.add_paths(&[d.to_path_buf()]);
        assert_eq!(report.added, vec![1]);
        assert_eq!(l.get(1).unwrap().source, d.join("a.docx"));

        let report = l.add_paths(&[d.join("~$a.docx"), d.join("._a.docx")]);
        assert_eq!(report.added, vec![2, 3]);
    }

    #[test]
    fn convert_flow_and_counts() {
        let mut l = list_with(&["a.docx", "b.docx"]);
        assert_eq!(l.convertible_count(), 2);
        let taken = l.take_for_convert();
        assert_eq!(taken.len(), 2);
        assert_eq!(labels(&l), ["Waiting", "Waiting"]);
        assert!(matches!(l.get(1).unwrap().status, FileStatus::Queued));
        assert!(l.is_batch_running());

        l.apply(WorkerEvent::Started(1));
        assert!(matches!(l.get(1).unwrap().status, FileStatus::Converting));
        l.apply(WorkerEvent::Finished(1, Ok(outcome())));
        assert!(matches!(l.get(1).unwrap().status, FileStatus::Done(_)));
        l.apply(WorkerEvent::Finished(2, Err(error())));
        assert!(matches!(l.get(2).unwrap().status, FileStatus::Failed(_)));
        l.apply(WorkerEvent::Started(99));

        assert_eq!(l.convertible_count(), 1);
        assert!(!l.is_batch_running());
    }

    #[test]
    fn retry_only_failed() {
        let mut l = list_with(&["a.docx", "b.docx"]);
        l.apply(WorkerEvent::Finished(1, Err(error())));
        l.apply(WorkerEvent::Finished(2, Ok(outcome())));
        let got = l.retry(1).unwrap();
        assert_eq!(got, (1, PathBuf::from("/nowhere/a.docx")));
        assert!(matches!(l.get(1).unwrap().status, FileStatus::Queued));
        assert!(l.retry(2).is_none());
        assert!(l.retry(1).is_none());
    }

    #[test]
    fn remove_only_waiting_or_queued() {
        let mut l = list_with(&["a.docx", "b.docx", "c.docx"]);
        l.queue_ids(&[2]);
        l.apply(WorkerEvent::Started(3));
        assert!(!l.remove(3));
        assert!(l.remove(1));
        assert!(l.remove(2));
        assert!(!l.remove(1));
        assert_eq!(l.items().len(), 1);
    }

    #[test]
    fn clear_keeps_waiting_and_running() {
        let mut l = list_with(&["1.docx", "2.docx", "3.docx", "4.docx", "5.docx"]);
        l.queue_ids(&[2, 3, 4, 5]);
        l.apply(WorkerEvent::Started(3));
        l.apply(WorkerEvent::Finished(4, Ok(outcome())));
        l.apply(WorkerEvent::Finished(5, Err(error())));
        l.clear_finished();
        let ids: Vec<_> = l.items().iter().map(|i| i.id).collect();
        assert_eq!(ids, [1, 2, 3]);
        assert!(matches!(l.get(2).unwrap().status, FileStatus::Queued));
        assert!(matches!(l.get(3).unwrap().status, FileStatus::Converting));
    }

    #[test]
    fn queue_ids_only_moves_waiting() {
        let mut l = list_with(&["a.docx"]);
        l.take_for_convert();
        let report = l.add_paths(&[PathBuf::from("/nowhere/b.docx")]);
        let queued = l.queue_ids(&report.added);
        assert_eq!(queued, vec![(2, PathBuf::from("/nowhere/b.docx"))]);
        assert!(matches!(l.get(2).unwrap().status, FileStatus::Queued));
        assert!(l.queue_ids(&[1, 2]).is_empty());
    }
}
