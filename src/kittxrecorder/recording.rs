//! One application-owned recording thread; the bridge/storage library stays synchronous.
use crate::{
    diagnostics::{DiagnosticConnector, DiagnosticSource, SessionLog, describe_error},
    model::{CaptureSettings, recording_path},
};
use kittx::{Client, ClientOptions, Initialization, Protocol, datastore::*};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Preparing,
    Recording,
    Stopping,
    Completed,
    Failed,
}
#[derive(Debug, Clone)]
pub struct LiveStatus {
    pub phase: Phase,
    pub path: PathBuf,
    pub progress: RecordingProgress,
    pub vin: Option<String>,
    pub message: String,
    pub summary: Option<RecordingSummary>,
    pub elapsed: Duration,
    /// Diagnostic sidecar for native recording jobs; pure/fake sessions may omit it.
    pub diagnostics_path: Option<PathBuf>,
    /// Latest recording/source failure (including partial peers) or log warning.
    pub last_error: Option<String>,
}
/// Bounded latest-value slot; progress never accumulates in a queue.
pub struct SessionControl {
    stop: AtomicBool,
    status: Mutex<LiveStatus>,
    began: Instant,
    log: Option<Arc<SessionLog>>,
    before_disconnect: Option<Arc<dyn Fn() + Send + Sync>>,
}
impl SessionControl {
    pub fn new(path: PathBuf) -> Self {
        Self {
            stop: AtomicBool::new(false),
            status: Mutex::new(LiveStatus {
                phase: Phase::Preparing,
                path,
                progress: RecordingProgress::default(),
                vin: None,
                message: "Connecting, reading VIN, discovering PIDs, and taking initial DTC scans…"
                    .into(),
                summary: None,
                elapsed: Duration::ZERO,
                diagnostics_path: None,
                last_error: None,
            }),
            began: Instant::now(),
            log: None,
            before_disconnect: None,
        }
    }
    pub fn request_stop(&self) {
        let first_request = !self.stop.swap(true, Ordering::AcqRel);
        let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
        let active = !matches!(s.phase, Phase::Completed | Phase::Failed);
        if active {
            s.phase = Phase::Stopping;
            s.message = "Stopping after preparation/current reads and final DTC scans…".into();
        }
        drop(s);
        if first_request && active {
            self.log("Stop requested by caller");
        }
    }
    pub fn snapshot(&self) -> LiveStatus {
        let mut s = self
            .status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if !matches!(s.phase, Phase::Completed | Phase::Failed) {
            s.elapsed = self.began.elapsed();
        }
        s
    }
    fn edit(&self, f: impl FnOnce(&mut LiveStatus)) {
        let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
        s.elapsed = self.began.elapsed();
        f(&mut s);
    }
    pub(crate) fn note_error(&self, message: String) {
        self.edit(|s| s.last_error = Some(message));
    }
    pub(crate) fn before_disconnect(&self) {
        if let Some(hook) = &self.before_disconnect {
            hook();
        }
    }
    fn log(&self, message: &str) {
        if let Some(log) = &self.log {
            log.info(message);
        }
    }
}
fn completed<T>(invoke: impl FnOnce(&mut dyn FnMut(DataResult<T>))) -> DataResult<T> {
    let mut result = None;
    invoke(&mut |r| result = Some(r));
    result.unwrap_or_else(|| {
        Err(DataError::State(
            "Recording callback was not invoked".into(),
        ))
    })
}
/// Shared execution path for the native app and deterministic fake-source tests.
pub fn run_session<S: DataSource, C: Clock>(
    source: S,
    clock: C,
    settings: CaptureSettings,
    path: PathBuf,
    control: &SessionControl,
) -> Result<RecordingSummary, String> {
    let final_scan_selected = settings.dtcs.iter().any(|on| *on) || settings.profile.is_some();
    let result = (|| {
        let config = settings.config()?;
        let mut recorder =
            Recorder::with_clock(source, config, clock).map_err(|e| describe_error(&e))?;
        control.log("Preparing recording: connecting, VIN, PID support, initial DTC scans");
        completed(|cb| recorder.start(&path, cb)).map_err(|e| describe_error(&e))?;
        let vin = RecordingReader::open(&path)
            .ok()
            .and_then(|r| r.session().ok())
            .and_then(|s| s.vin);
        control.edit(|s| {
            s.phase = if control.stop.load(Ordering::Acquire) {
                Phase::Stopping
            } else {
                Phase::Recording
            };
            s.vin = vin;
            s.progress = recorder.progress().clone();
            s.message = if control.stop.load(Ordering::Acquire) {
                "Stopping after preparation/current reads and final DTC scans…".into()
            } else {
                "Recording; writes are committed after every acquisition".into()
            };
        });
        control.log("Preparation complete; acquiring selected signals");
        let result = completed(|cb| {
            recorder.run(
                |progress| {
                    control.edit(|s| s.progress = progress.clone());
                    if control.stop.load(Ordering::Acquire) {
                        RecordControl::Stop
                    } else {
                        RecordControl::Continue
                    }
                },
                cb,
            )
        });
        match result {
            Ok(summary) => Ok(summary),
            Err(error) => {
                let original = describe_error(&error);
                let finish = completed(|cb| recorder.finish(cb));
                match finish {
                    Ok(summary) => {
                        control.edit(|s| {
                            s.progress = summary.progress.clone();
                            s.summary = Some(summary);
                        });
                        Err(format!(
                            "Capture failed: {original}. The committed file was finalized."
                        ))
                    }
                    Err(finish) => Err(format!(
                        "Capture failed: {original}. Finalization: {finish}. Keep the file and any WAL sidecars."
                    )),
                }
            }
        }
    })();
    control.edit(|s| match &result {
        Ok(summary) => {
            s.phase = Phase::Completed;
            s.progress = summary.progress.clone();
            s.summary = Some(summary.clone());
            s.message = if summary.ending_scan_complete || !final_scan_selected {
                "Recording finalized".into()
            } else {
                "Recording finalized; final DTC coverage is incomplete".into()
            };
        }
        Err(error) => {
            s.phase = Phase::Failed;
            s.message = error.clone();
        }
    });
    match &result {
        Ok(summary) => control.log(&format!("Recording finalized: {summary:?}")),
        Err(error) => {
            control.edit(|s| {
                if s.last_error.is_none() {
                    s.last_error = Some(error.clone());
                }
            });
            if let Some(log) = &control.log {
                log.failure(&format!("Recording failed: {error}"));
            }
        }
    }
    result
}
/// Owns at most one worker and its stop token. Dropping requests stop; it never blocks the UI.
pub struct RecordingJob {
    control: Arc<SessionControl>,
    handle: Option<JoinHandle<()>>,
}
impl RecordingJob {
    pub fn spawn(settings: CaptureSettings, path: PathBuf) -> Result<Self, String> {
        Self::spawn_inner(settings, path, None)
    }
    /// Application-owned barrier before disconnect/finalization. Runs on the
    /// recording worker, including failed-start cleanup. Live-view applications
    /// can stop new SQLite readers and wait for existing reads to release their
    /// connections before WAL checkpoint/journal-mode finalization. Never call
    /// back into the recording job or block on UI work from this hook.
    pub fn spawn_with_disconnect_hook(
        settings: CaptureSettings,
        path: PathBuf,
        hook: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, String> {
        Self::spawn_inner(settings, path, Some(hook))
    }
    fn spawn_inner(
        settings: CaptureSettings,
        path: PathBuf,
        hook: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Result<Self, String> {
        settings.config()?;
        let path = recording_path(&path)?;
        let debug = std::env::var("KITTX_RECORDER_LOG")
            .is_ok_and(|level| level.eq_ignore_ascii_case("debug"));
        let (log, log_error) = match SessionLog::open(&path, debug) {
            Ok(log) => (log, None),
            Err(error) => (
                SessionLog::console(&path, debug),
                Some(format!(
                    "Cannot open recording diagnostic log: {error}; using console diagnostics"
                )),
            ),
        };
        log.info(&format!(
            "{} {} session {}; debug={debug}; settings={settings:?}",
            settings.producer,
            env!("CARGO_PKG_VERSION"),
            path.display()
        ));
        let mut state = SessionControl::new(path.clone());
        state.before_disconnect = hook;
        state.log = Some(log.clone());
        state.edit(|s| {
            if log_error.is_none() {
                s.diagnostics_path = Some(log.path.clone());
            }
            s.last_error = log_error.clone();
        });
        if let Some(error) = log_error {
            log.failure(&error);
        }
        let control = Arc::new(state);
        let worker = control.clone();
        let handle = thread::Builder::new()
            .name("kittx-recording".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let client = Client::with_connector(
                        settings.transport.clone(),
                        ClientOptions::default(),
                        Initialization::obd(Protocol::Automatic),
                        Box::new(DiagnosticConnector { log: log.clone() }),
                    )
                    .map_err(|e| describe_error(&e))?;
                    let source = DiagnosticSource {
                        inner: ClientSource::new(client),
                        log: log.clone(),
                        control: worker.clone(),
                    };
                    run_session(source, SystemClock::default(), settings, path, &worker)
                }));
                match result {
                    Ok(Err(error)) => worker.edit(|s| {
                        s.phase = Phase::Failed;
                        s.message = error;
                    }),
                    Err(panic) => {
                        let reason = panic
                            .downcast_ref::<String>()
                            .map(String::as_str)
                            .or_else(|| panic.downcast_ref::<&str>().copied())
                            .unwrap_or("unknown panic");
                        let detail = format!(
                            "Recording worker panicked: {reason}; committed data remains in the file. Preserve WAL sidecars."
                        );
                        log.failure(&detail);
                        worker.note_error(detail.clone());
                        worker.edit(|s| {
                            s.phase = Phase::Failed;
                            s.message = detail;
                        });
                    }
                    Ok(Ok(_)) => {}
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            control,
            handle: Some(handle),
        })
    }
    pub fn request_stop(&self) {
        self.control.request_stop();
    }
    pub fn snapshot(&self) -> LiveStatus {
        self.control.snapshot()
    }
    pub fn is_finished(&self) -> bool {
        self.handle.as_ref().is_none_or(|h| h.is_finished())
    }
    /// Call only after is_finished; it will not wait on an active worker.
    pub fn reap(&mut self) -> bool {
        if !self.is_finished() {
            return false;
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        true
    }
}
impl Drop for RecordingJob {
    fn drop(&mut self) {
        self.request_stop();
    }
}
