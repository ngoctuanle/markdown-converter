//! Converts one office document to Markdown with pandoc.

use crate::ast::normalize;
use crate::formats::InputFormat;
use crate::media::{MediaContext, rewrite_images};
use crate::naming::{Reservation, reserve_output};
use crate::pandoc::{Pandoc, PandocError};
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct ConvertRequest {
    pub source: PathBuf,
    pub output_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ConvertOutcome {
    pub md_path: PathBuf,
    /// `Some` only when at least one picture was copied.
    pub assets_dir: Option<PathBuf>,
    pub pictures: usize,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ConvertError {
    /// One line for the file row.
    pub reason: String,
    /// Full text for the details view.
    pub detail: String,
}

impl ConvertError {
    fn new(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        ConvertError {
            detail: reason.clone(),
            reason,
        }
    }
}

impl From<PandocError> for ConvertError {
    fn from(e: PandocError) -> Self {
        ConvertError {
            reason: e.short_reason(),
            detail: e.detail(),
        }
    }
}

/// Converts `req.source` into `<stem>-converted.md` (plus an assets folder
/// when it has pictures) in `req.output_dir`. On failure nothing is left behind.
pub fn convert_file(pandoc: &Pandoc, req: &ConvertRequest) -> Result<ConvertOutcome, ConvertError> {
    let format = InputFormat::from_path(&req.source)
        .ok_or_else(|| ConvertError::new("Unsupported file type"))?;
    if !req.source.exists() {
        return Err(ConvertError::new("File not found"));
    }
    if !req.output_dir.is_dir() {
        return Err(ConvertError::new(format!(
            "Output folder not found: {}",
            req.output_dir.display()
        )));
    }
    let stem = req
        .source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let reservation = reserve_output(&req.output_dir, &stem)
        .map_err(|e| ConvertError::new(format!("Cannot write to output folder: {e}")))?;

    let mut reservation = reservation;
    steps(pandoc, req, format, &mut reservation).inspect_err(|_| {
        let _ = reservation.discard();
    })
}

fn steps(
    pandoc: &Pandoc,
    req: &ConvertRequest,
    format: InputFormat,
    reservation: &mut Reservation,
) -> Result<ConvertOutcome, ConvertError> {
    let write_err = |e: std::io::Error| ConvertError::new(format!("Could not write output: {e}"));

    let tmp = tempfile::TempDir::new()
        .map_err(|e| ConvertError::new(format!("Could not create a temporary folder: {e}")))?;
    let mut extract = OsString::from("--extract-media=");
    extract.push(tmp.path());
    let args: Vec<OsString> = vec![
        "--sandbox".into(),
        "-f".into(),
        format.reader().into(),
        "-t".into(),
        "json".into(),
        extract,
        req.source.clone().into_os_string(),
    ];
    let json = pandoc.run(&args, None)?;
    let mut ast: serde_json::Value = serde_json::from_slice(&json)
        .map_err(|e| ConvertError::new(format!("Could not read pandoc output: {e}")))?;
    // Leaves complex tables as the only HTML the gfm writer can emit.
    normalize(&mut ast);

    let source_dir = req
        .source
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let prefix = reservation.assets_dir_name();
    let report = rewrite_images(
        &mut ast,
        &MediaContext {
            extract_dir: tmp.path(),
            source_dir: &source_dir,
            assets_dir: &reservation.assets_dir,
            link_prefix: &prefix,
        },
    )
    .map_err(write_err)?;

    let stdin = serde_json::to_vec(&ast)
        .map_err(|e| ConvertError::new(format!("Could not write pandoc input: {e}")))?;
    let args: Vec<OsString> = ["--sandbox", "-f", "json", "-t", "gfm", "--wrap=none"]
        .into_iter()
        .map(OsString::from)
        .collect();
    let markdown = pandoc.run(&args, Some(stdin))?;
    reservation.write_markdown(&markdown).map_err(write_err)?;

    Ok(ConvertOutcome {
        md_path: reservation.md_path.clone(),
        assets_dir: (report.pictures > 0).then(|| reservation.assets_dir.clone()),
        pictures: report.pictures,
        warnings: report.warnings,
    })
}
