//! Runs conversions on a pool of worker threads and reports progress as events.

use crate::convert::{ConvertError, ConvertOutcome, ConvertRequest, convert_file};
use crate::pandoc::Pandoc;
use smol::channel::{Receiver, Sender};
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex};

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
}

type Shared = Arc<(Mutex<State>, Condvar)>;

pub struct Converter {
    shared: Shared,
}

impl Converter {
    pub fn default_workers() -> usize {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    }

    pub fn start(pandoc: Pandoc) -> (Converter, Receiver<WorkerEvent>) {
        Self::start_with(
            Self::default_workers(),
            Arc::new(move |req| convert_file(&pandoc, req)),
        )
    }

    pub fn start_with(workers: usize, f: ConvertFn) -> (Converter, Receiver<WorkerEvent>) {
        let shared: Shared = Arc::new((
            Mutex::new(State {
                jobs: VecDeque::new(),
                shutdown: false,
            }),
            Condvar::new(),
        ));
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
        (Converter { shared }, rx)
    }

    pub fn enqueue(&self, id: JobId, req: ConvertRequest) {
        let (state, wake) = &*self.shared;
        state.lock().unwrap().jobs.push_back((id, req));
        wake.notify_one();
    }

    /// True only if the job was still queued and has been removed.
    pub fn cancel(&self, id: JobId) -> bool {
        let mut state = self.shared.0.lock().unwrap();
        match state.jobs.iter().position(|(job, _)| *job == id) {
            Some(pos) => {
                state.jobs.remove(pos);
                true
            }
            None => false,
        }
    }
}

impl Drop for Converter {
    /// Does not join: workers finish their current job and exit. Jobs still
    /// queued at drop are abandoned and get no `Finished` event.
    fn drop(&mut self) {
        let (state, wake) = &*self.shared;
        state.lock().unwrap().shutdown = true;
        wake.notify_all();
    }
}

fn worker_loop(shared: &Shared, f: &ConvertFn, tx: &Sender<WorkerEvent>) {
    let (state, wake) = &**shared;
    loop {
        let (id, req) = {
            let mut guard = state.lock().unwrap();
            loop {
                if guard.shutdown {
                    return;
                }
                if let Some(job) = guard.jobs.pop_front() {
                    break job;
                }
                guard = wake.wait(guard).unwrap();
            }
        };
        // A send error means the receiver (the UI) is gone.
        if tx.send_blocking(WorkerEvent::Started(id)).is_err() {
            return;
        }
        // One panicking conversion must not kill the worker or lose its event.
        let result = catch_unwind(AssertUnwindSafe(|| f(&req))).unwrap_or_else(|payload| {
            let detail = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            Err(ConvertError {
                reason: "Internal error while converting".into(),
                detail,
            })
        });
        if tx.send_blocking(WorkerEvent::Finished(id, result)).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
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
                    assert_eq!(state.insert(id, 2), Some(1), "job {id} finished before start");
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
        assert!(matches!(rx.recv_blocking().unwrap(), WorkerEvent::Started(1)));
        assert!(converter.cancel(2));
        assert!(!converter.cancel(1));
        assert!(matches!(
            rx.recv_blocking().unwrap(),
            WorkerEvent::Finished(1, Ok(_))
        ));
        let extra = smol::block_on(smol::future::or(
            async { rx.recv().await.ok() },
            async {
                smol::Timer::after(Duration::from_secs(1)).await;
                None
            },
        ));
        assert!(extra.is_none(), "unexpected extra event");
    }

    #[test]
    fn panicking_conversion_is_reported_and_worker_survives() {
        let f: ConvertFn = Arc::new(|req| {
            if req.source == PathBuf::from("file-1.docx") {
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
    fn default_workers_at_least_one() {
        assert!(Converter::default_workers() >= 1);
    }
}
