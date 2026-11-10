//! Runs conversions on a pool of worker threads and reports progress as events.

use crate::convert::{ConvertError, ConvertOutcome, ConvertRequest, convert_file};
use crate::pandoc::Pandoc;
use smol::channel::{Receiver, Sender};
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub type JobId = u64;

#[derive(Debug)]
pub enum WorkerEvent {
    Started(JobId),
    Finished(JobId, Result<ConvertOutcome, ConvertError>),
}

pub type ConvertFn =
    Arc<dyn Fn(&ConvertRequest) -> Result<ConvertOutcome, ConvertError> + Send + Sync>;

struct State {
    jobs: VecDeque<(JobId, ConvertRequest)>,
    shutdown: bool,
    /// Jobs a worker has taken and not finished yet.
    active: usize,
}

struct Shared {
    state: Mutex<State>,
    /// Signalled when a job is queued or on shutdown.
    wake: Condvar,
    /// Signalled when a worker finishes a job.
    done: Condvar,
}

pub struct Converter {
    shared: Arc<Shared>,
    /// Killed on shutdown; `None` for the stub conversions used in tests.
    pandoc: Option<Pandoc>,
}

impl Converter {
    pub fn default_workers() -> usize {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    }

    pub fn start(pandoc: Pandoc) -> (Converter, Receiver<WorkerEvent>) {
        let runner = pandoc.clone();
        let (mut converter, rx) = Self::start_with(
            Self::default_workers(),
            Arc::new(move |req| convert_file(&runner, req)),
        );
        converter.pandoc = Some(pandoc);
        (converter, rx)
    }

    pub fn start_with(workers: usize, f: ConvertFn) -> (Converter, Receiver<WorkerEvent>) {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                jobs: VecDeque::new(),
                shutdown: false,
                active: 0,
            }),
            wake: Condvar::new(),
            done: Condvar::new(),
        });
        let (tx, rx) = smol::channel::unbounded();
        for i in 0..workers.max(1) {
            let shared = shared.clone();
            let f = f.clone();
            let tx = tx.clone();
            std::thread::Builder::new()
                .name(format!("convert-{i}"))
                .spawn(move || worker_loop(&shared, &f, &tx))
                .expect("failed to spawn conversion worker");
        }
        (
            Converter {
                shared,
                pandoc: None,
            },
            rx,
        )
    }

    pub fn enqueue(&self, id: JobId, req: ConvertRequest) {
        self.shared.state.lock().unwrap().jobs.push_back((id, req));
        self.shared.wake.notify_one();
    }

    /// True only if the job was still queued and has been removed.
    pub fn cancel(&self, id: JobId) -> bool {
        let mut state = self.shared.state.lock().unwrap();
        match state.jobs.iter().position(|(job, _)| *job == id) {
            Some(pos) => {
                state.jobs.remove(pos);
                true
            }
            None => false,
        }
    }

    /// Stops for good, for when the app quits: drops the queued jobs, kills
    /// the running pandoc processes, then waits up to `wait` for the jobs in
    /// progress to unwind (deleting their partial output as they do).
    /// Returns true if no job is still running.
    pub fn shutdown(&self, wait: Duration) -> bool {
        {
            let mut state = self.shared.state.lock().unwrap();
            state.shutdown = true;
            state.jobs.clear();
        }
        self.shared.wake.notify_all();
        if let Some(pandoc) = &self.pandoc {
            pandoc.kill_all_running();
        }
        let state = self.shared.state.lock().unwrap();
        let (state, _) = self
            .shared
            .done
            .wait_timeout_while(state, wait, |s| s.active > 0)
            .unwrap();
        state.active == 0
    }
}

impl Drop for Converter {
    /// Does not join: workers finish their current job and exit. Jobs still
    /// queued at drop are abandoned and get no `Finished` event.
    fn drop(&mut self) {
        self.shared.state.lock().unwrap().shutdown = true;
        self.shared.wake.notify_all();
    }
}

fn worker_loop(shared: &Shared, f: &ConvertFn, tx: &Sender<WorkerEvent>) {
    loop {
        let (id, req) = {
            let mut guard = shared.state.lock().unwrap();
            loop {
                if guard.shutdown {
                    return;
                }
                if let Some(job) = guard.jobs.pop_front() {
                    guard.active += 1;
                    break job;
                }
                guard = shared.wake.wait(guard).unwrap();
            }
        };
        // A send error means the receiver (the UI) is gone: skip the job.
        let result = tx
            .send_blocking(WorkerEvent::Started(id))
            .ok()
            .map(|()| run_job(f, &req));
        shared.state.lock().unwrap().active -= 1;
        shared.done.notify_all();
        let Some(result) = result else { return };
        if tx.send_blocking(WorkerEvent::Finished(id, result)).is_err() {
            return;
        }
    }
}

/// Runs one conversion. A panic becomes an error, so one panicking conversion
/// doesn't kill the worker or lose its event.
fn run_job(f: &ConvertFn, req: &ConvertRequest) -> Result<ConvertOutcome, ConvertError> {
    catch_unwind(AssertUnwindSafe(|| f(req))).unwrap_or_else(|payload| {
        let detail = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        Err(ConvertError {
            reason: "Internal error while converting".into(),
            detail,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn stub(delay_ms: u64, live: Arc<AtomicUsize>, peak: Arc<AtomicUsize>) -> ConvertFn {
        Arc::new(move |req: &ConvertRequest| {
            let now = live.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(delay_ms));
            live.fetch_sub(1, Ordering::SeqCst);
            Ok(ConvertOutcome {
                md_path: req.source.clone(),
                assets_dir: None,
                pictures: 0,
                warnings: Vec::new(),
            })
        })
    }

    fn request(n: u64) -> ConvertRequest {
        ConvertRequest {
            source: PathBuf::from(format!("file-{n}.docx")),
            output_dir: PathBuf::from("out"),
        }
    }

    #[test]
    fn runs_at_most_n_at_once_and_all_finish() {
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (converter, rx) = Converter::start_with(3, stub(50, live, peak.clone()));
        for id in 1..=10 {
            converter.enqueue(id, request(id));
        }
        // Per job: 0 = nothing yet, 1 = started, 2 = finished.
        let mut state: HashMap<JobId, u8> = HashMap::new();
        for _ in 0..20 {
            match rx.recv_blocking().unwrap() {
                WorkerEvent::Started(id) => {
                    assert_eq!(state.insert(id, 1), None, "job {id} started twice");
                }
                WorkerEvent::Finished(id, result) => {
                    assert_eq!(
                        state.insert(id, 2),
                        Some(1),
                        "job {id} finished before start"
                    );
                    assert_eq!(result.unwrap().md_path, request(id).source);
                }
            }
        }
        assert_eq!(state.len(), 10);
        assert!(state.values().all(|&s| s == 2));
        assert_eq!(peak.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn cancel_removes_queued_job() {
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (converter, rx) = Converter::start_with(1, stub(200, live, peak));
        converter.enqueue(1, request(1));
        converter.enqueue(2, request(2));
        assert!(matches!(
            rx.recv_blocking().unwrap(),
            WorkerEvent::Started(1)
        ));
        assert!(converter.cancel(2));
        assert!(!converter.cancel(1));
        assert!(matches!(
            rx.recv_blocking().unwrap(),
            WorkerEvent::Finished(1, Ok(_))
        ));
        let extra = smol::block_on(smol::future::or(async { rx.recv().await.ok() }, async {
            smol::Timer::after(Duration::from_secs(1)).await;
            None
        }));
        assert!(extra.is_none(), "unexpected extra event");
    }

    #[test]
    fn panicking_conversion_is_reported_and_worker_survives() {
        let f: ConvertFn = Arc::new(|req| {
            if req.source == Path::new("file-1.docx") {
                panic!("boom");
            }
            Ok(ConvertOutcome {
                md_path: req.source.clone(),
                assets_dir: None,
                pictures: 0,
                warnings: Vec::new(),
            })
        });
        let (converter, rx) = Converter::start_with(1, f);
        converter.enqueue(1, request(1));
        converter.enqueue(2, request(2));
        let mut events = Vec::new();
        for _ in 0..4 {
            events.push(rx.recv_blocking().unwrap());
        }
        assert!(matches!(events[0], WorkerEvent::Started(1)));
        match &events[1] {
            WorkerEvent::Finished(1, Err(e)) => {
                assert_eq!(e.reason, "Internal error while converting");
                assert_eq!(e.detail, "boom");
            }
            other => panic!("unexpected event {other:?}"),
        }
        assert!(matches!(events[2], WorkerEvent::Started(2)));
        assert!(matches!(events[3], WorkerEvent::Finished(2, Ok(_))));
    }

    #[test]
    fn shutdown_drops_queued_jobs_and_waits_for_running_one() {
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (converter, rx) = Converter::start_with(1, stub(300, live.clone(), peak));
        for id in 1..=3 {
            converter.enqueue(id, request(id));
        }
        assert!(matches!(
            rx.recv_blocking().unwrap(),
            WorkerEvent::Started(1)
        ));
        assert!(converter.shutdown(Duration::from_secs(2)));
        assert_eq!(live.load(Ordering::SeqCst), 0, "job 1 still running");
        drop(converter);
        let mut rest = Vec::new();
        while let Ok(ev) = rx.recv_blocking() {
            rest.push(ev);
        }
        assert!(
            matches!(rest.as_slice(), [WorkerEvent::Finished(1, Ok(_))]),
            "{rest:?}"
        );
    }

    #[test]
    fn shutdown_wait_is_bounded() {
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (converter, rx) = Converter::start_with(1, stub(1500, live, peak));
        converter.enqueue(1, request(1));
        assert!(matches!(
            rx.recv_blocking().unwrap(),
            WorkerEvent::Started(1)
        ));
        let t = std::time::Instant::now();
        assert!(!converter.shutdown(Duration::from_millis(100)));
        assert!(t.elapsed() < Duration::from_millis(1000));
    }

    /// Quitting mid-conversion kills pandoc and leaves no reserved output behind.
    #[cfg(unix)]
    #[test]
    fn shutdown_kills_pandoc_and_removes_reserved_output() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let fake = d.path().join("pandoc");
        std::fs::write(&fake, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = d.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let source = out.join("report.docx");
        std::fs::write(&source, b"x").unwrap();

        let (converter, rx) = Converter::start(Pandoc::at(fake));
        converter.enqueue(
            1,
            ConvertRequest {
                source,
                output_dir: out.clone(),
            },
        );
        assert!(matches!(
            rx.recv_blocking().unwrap(),
            WorkerEvent::Started(1)
        ));
        // Give the job time to reserve its output and start pandoc.
        std::thread::sleep(Duration::from_millis(300));
        assert!(out.join("report-converted.md").exists());
        let t = std::time::Instant::now();
        assert!(converter.shutdown(Duration::from_secs(2)));
        assert!(t.elapsed() < Duration::from_secs(1));
        let names: Vec<_> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["report.docx"]);
    }

    #[test]
    fn default_workers_at_least_one() {
        assert!(Converter::default_workers() >= 1);
    }
}
