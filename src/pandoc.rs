//! Find the bundled pandoc and run it with piped stdin/stdout and a timeout.

use std::ffi::OsString;
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use wait_timeout::ChildExt;

/// The pandoc release the app is built and tested against.
pub const PINNED_VERSION: &str = "3.13";

/// How long one pandoc run may take before it is killed.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// Longest `short_reason`, in bytes.
const MAX_REASON_LEN: usize = 120;

#[cfg(windows)]
const BIN: &str = "pandoc.exe";
#[cfg(not(windows))]
const BIN: &str = "pandoc";

/// A pandoc executable plus the timeout applied to each run.
#[derive(Clone, Debug)]
pub struct Pandoc {
    path: PathBuf,
    timeout: Duration,
}

#[derive(Debug)]
pub enum PandocError {
    NotFound,
    Spawn(std::io::Error),
    Timeout(Duration),
    Failed { code: Option<i32>, stderr: String },
}

impl Pandoc {
    /// Finds pandoc: `MDCONV_PANDOC`, then next to the executable and in the
    /// macOS, Linux-deb and generic resource folders; in debug builds also the
    /// `vendor/pandoc/<target>` copy and finally `pandoc` on `PATH`.
    pub fn locate() -> Result<Pandoc, PandocError> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(p) = std::env::var_os("MDCONV_PANDOC") {
            candidates.push(p.into());
        }
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_path_buf))
        {
            candidates.push(dir.join(BIN));
            candidates.push(dir.join("../Resources").join(BIN));
            candidates.push(dir.join("../lib/markdown-converter").join(BIN));
            candidates.push(dir.join("resources").join(BIN));
        }
        #[cfg(debug_assertions)]
        candidates.push(
            Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/vendor/pandoc/",
                env!("MDCONV_TARGET")
            ))
            .join(BIN),
        );

        if let Some(found) = candidates.into_iter().find(|p| p.is_file()) {
            return Ok(Pandoc::at(found));
        }

        #[cfg(debug_assertions)]
        {
            let on_path = Pandoc::at(PathBuf::from(BIN));
            if on_path.run(&["--version".into()], None).is_ok() {
                return Ok(on_path);
            }
        }

        Err(PandocError::NotFound)
    }

    /// Uses the pandoc at `path`, with the default timeout.
    pub fn at(path: PathBuf) -> Pandoc {
        Pandoc {
            path,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_timeout(self, t: Duration) -> Pandoc {
        Pandoc { timeout: t, ..self }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Runs pandoc with `args`, feeding `stdin` if given, and returns stdout.
    /// The process is killed if it runs longer than the timeout.
    pub fn run(&self, args: &[OsString], stdin: Option<Vec<u8>>) -> Result<Vec<u8>, PandocError> {
        let mut cmd = Command::new(&self.path);
        cmd.args(args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(PandocError::Spawn)?;

        if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
            // A write error means pandoc exited early; its exit status says why.
            thread::spawn(move || {
                let _ = pipe.write_all(&bytes);
            });
        }
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());

        let status = match child.wait_timeout(self.timeout) {
            Ok(Some(status)) => status,
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                // Don't join the readers: a grandchild may still hold the pipes.
                return Err(PandocError::Timeout(self.timeout));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(PandocError::Spawn(e));
            }
        };

        let out = stdout.join().unwrap_or_default();
        let err = stderr.join().unwrap_or_default();
        if status.success() {
            Ok(out)
        } else {
            Err(PandocError::Failed {
                code: status.code(),
                stderr: String::from_utf8_lossy(&err).into_owned(),
            })
        }
    }
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    })
}

impl PandocError {
    /// One line for the file row, at most 120 bytes.
    pub fn short_reason(&self) -> String {
        match self {
            Self::NotFound => "Bundled pandoc not found".into(),
            Self::Spawn(e) => truncate(&format!("Could not start pandoc: {e}")),
            Self::Timeout(d) => {
                let secs = d.as_secs();
                if secs >= 60 && secs % 60 == 0 {
                    format!("Timed out after {} min", secs / 60)
                } else {
                    format!("Timed out after {:.1} s", d.as_secs_f64())
                }
            }
            Self::Failed { code, stderr } => {
                match stderr.lines().map(str::trim).find(|l| !l.is_empty()) {
                    Some(line) => truncate(line),
                    None => match code {
                        Some(c) => format!("pandoc exited with code {c}"),
                        None => "pandoc was terminated by a signal".into(),
                    },
                }
            }
        }
    }

    /// Full text for the preview pane: all of stderr, else the short reason.
    pub fn detail(&self) -> String {
        match self {
            Self::Failed { stderr, .. } if !stderr.trim().is_empty() => {
                stderr.trim_end().to_string()
            }
            _ => self.short_reason(),
        }
    }
}

impl fmt::Display for PandocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.short_reason())
    }
}

impl std::error::Error for PandocError {}

/// Cuts `s` to at most `MAX_REASON_LEN` bytes on a char boundary, adding `…`.
fn truncate(s: &str) -> String {
    if s.len() <= MAX_REASON_LEN {
        return s.to_string();
    }
    let mut end = MAX_REASON_LEN - '…'.len_utf8();
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locate_finds_a_pandoc_in_dev() {
        let p = Pandoc::locate().expect("run scripts/fetch-pandoc.sh first");
        let out = p.run(&["--version".into()], None).unwrap();
        assert!(String::from_utf8_lossy(&out).starts_with(&format!("pandoc {PINNED_VERSION}")));
    }

    #[test]
    fn run_pipes_stdin_to_stdout() {
        let p = Pandoc::locate().unwrap();
        let out = p
            .run(
                &["-f".into(), "markdown".into(), "-t".into(), "gfm".into()],
                Some(b"# Hi\n".to_vec()),
            )
            .unwrap();
        assert_eq!(String::from_utf8(out).unwrap().trim(), "# Hi");
    }

    #[test]
    fn failure_reports_first_stderr_line() {
        let p = Pandoc::locate().unwrap();
        let e = p
            .run(&["-f".into(), "nonsense-format".into()], Some(vec![]))
            .unwrap_err();
        assert!(matches!(e, PandocError::Failed { .. }));
        assert!(!e.short_reason().is_empty() && e.short_reason().len() <= 120);
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_process() {
        let d = tempfile::tempdir().unwrap();
        let fake = d.path().join("pandoc");
        std::fs::write(&fake, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let t = std::time::Instant::now();
        let e = Pandoc::at(fake)
            .with_timeout(Duration::from_millis(300))
            .run(&[], None)
            .unwrap_err();
        assert!(matches!(e, PandocError::Timeout(_)));
        assert!(t.elapsed() < Duration::from_secs(5));
        assert_eq!(
            PandocError::Timeout(DEFAULT_TIMEOUT).short_reason(),
            "Timed out after 5 min"
        );
    }
}
