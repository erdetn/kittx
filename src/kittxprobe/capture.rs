//! Capture ownership and explicit save-after-stop. Working recordings are retained
//! in a unique temporary directory, including after errors, cancellation or exit.
use crate::live::LiveSnapshot;
use kittx::datastore::RecordingReader;
use kittx_recorder::{
    model::{CaptureSettings, recording_path},
    recording::{LiveStatus, RecordingJob},
};
use kittx_viewer::model::Viewport;
use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// One adapter worker and its recoverable working file. Dropping requests Stop
/// through the worker; it never deletes an unsaved recording or blocks the UI.
pub struct ProbeCapture {
    job: RecordingJob,
    saved_to: Option<PathBuf>,
    readers: Arc<ReadGate>,
}

#[derive(Default)]
struct ReaderState {
    count: usize,
    paused: bool,
}
#[derive(Default)]
struct ReadGate {
    state: Mutex<ReaderState>,
    released: Condvar,
}
struct ReadLease<'a>(&'a ReadGate);
impl ReadGate {
    fn enter(&self) -> Result<ReadLease<'_>, String> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.paused {
            return Err("Finalizing capture; waiting for the stopped snapshot".into());
        }
        state.count += 1;
        Ok(ReadLease(self))
    }
    fn pause(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.paused = true;
        while state.count > 0 {
            state = self.released.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
}
impl Drop for ReadLease<'_> {
    fn drop(&mut self) {
        self.0.state.lock().unwrap_or_else(|e| e.into_inner()).count -= 1;
        self.0.released.notify_all();
    }
}

/// Cloneable snapshot request for a background executor. Live requests hold a
/// reader lease; finalization waits for its release before closing the WAL.
#[derive(Clone)]
pub struct CaptureReader {
    path: PathBuf,
    readers: Arc<ReadGate>,
    finalized: bool,
}
impl CaptureReader {
    pub fn read(&self, rolling: bool, pixels: u32) -> Result<LiveSnapshot, String> {
        self.read_view(rolling, pixels, None)
    }

    /// Read an explicit plot interval under the same finalization lease as
    /// `read`. `None` selects the automatic rolling/full interval.
    pub fn read_view(
        &self,
        rolling: bool,
        pixels: u32,
        view: Option<Viewport>,
    ) -> Result<LiveSnapshot, String> {
        let _lease = if self.finalized {
            None
        } else {
            Some(self.readers.enter()?)
        };
        LiveSnapshot::read_view(&self.path, rolling, pixels, view)
    }
}

impl ProbeCapture {
    /// Validate before creating a working file, then connect on the recorder's
    /// single worker thread. Only the selected measurements/DTC types are read.
    pub fn start(mut settings: CaptureSettings) -> Result<Self, String> {
        settings.producer = "KittxProbe".into();
        settings.config()?;
        let root = std::env::temp_dir().join(format!(
            "kittx-probe-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&root).map_err(|e| format!("Create working directory: {e}"))?;
        let readers = Arc::new(ReadGate::default());
        let barrier = readers.clone();
        let job = RecordingJob::spawn_with_disconnect_hook(
            settings,
            root.join("capture.kittx"),
            Arc::new(move || barrier.pause()),
        )?;
        Ok(Self {
            job,
            saved_to: None,
            readers,
        })
    }

    pub fn snapshot(&self) -> LiveStatus {
        self.job.snapshot()
    }
    pub fn request_stop(&self) {
        self.job.request_stop();
    }
    pub fn is_finished(&self) -> bool {
        self.job.is_finished()
    }
    /// Nonblocking: joins only an already finished worker.
    pub fn reap(&mut self) -> bool {
        self.job.reap()
    }
    pub fn saved_to(&self) -> Option<&Path> {
        self.saved_to.as_deref()
    }
    pub fn reader(&self) -> CaptureReader {
        CaptureReader {
            path: self.snapshot().path,
            readers: self.readers.clone(),
            finalized: self.is_finished(),
        }
    }
    /// Failed captures with a readable committed prefix may also be saved.
    pub fn can_save(&self) -> bool {
        self.is_finished() && self.snapshot().path.is_file()
    }
    /// Save a standalone SQLite backup after the worker has stopped. No overwrite
    /// or schema/metadata changes; WAL contents are included. This is blocking
    /// disk I/O and belongs on an application background executor, not the UI.
    pub fn save(&mut self, destination: &Path) -> Result<PathBuf, String> {
        if !self.reap() {
            return Err("Stop the capture before saving".into());
        }
        let path = save_recording(&self.snapshot().path, destination)?;
        self.saved_to = Some(path.clone());
        Ok(path)
    }
}

fn save_recording(source: &Path, destination: &Path) -> Result<PathBuf, String> {
    // Validate the actual recording, including schema/application IDs.
    let info = RecordingReader::open(source)
        .and_then(|r| r.session())
        .map_err(|e| e.to_string())?;
    if matches!(
        info.status.as_str(),
        "preparing" | "recording" | "finalizing"
    ) {
        return Err("The working file is not finalized; keep it and any WAL sidecars".into());
    }
    let destination = recording_path(destination)?;
    // create_new closes the race after the save-dialog existence check.
    let reserved = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|e| format!("Create save file: {e}"))?;
    let result = (|| {
        let source = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| e.to_string())?;
        let mut target =
            Connection::open_with_flags(&destination, OpenFlags::SQLITE_OPEN_READ_WRITE)
                .map_err(|e| e.to_string())?;
        target
            .busy_timeout(Duration::from_millis(250))
            .map_err(|e| e.to_string())?;
        backup_database(&source, &mut target)?;
        target
            .pragma_update(None, "journal_mode", "DELETE")
            .map_err(|e| e.to_string())?;
        target.close().map_err(|(_, e)| e.to_string())?;
        reserved.sync_all().map_err(|e| e.to_string())?;
        RecordingReader::open(&destination).map_err(|e| e.to_string())?;
        Ok(destination.clone())
    })();
    drop(reserved);
    if result.is_err() {
        let _ = fs::remove_file(&destination);
    }
    result
}

fn backup_database(source: &Connection, target: &mut Connection) -> Result<(), String> {
    let backup = Backup::new(source, target).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if Instant::now() >= deadline {
            return Err(
                "Save exceeded its 60-second deadline; the working recording is retained".into(),
            );
        }
        match backup.step(256).map_err(|e| e.to_string())? {
            StepResult::Done => return Ok(()),
            StepResult::Busy | StepResult::Locked => std::thread::sleep(Duration::from_millis(5)),
            _ => (),
        }
    }
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
