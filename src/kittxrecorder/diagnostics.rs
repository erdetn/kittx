//! Application-owned diagnostics; bridge clients remain independent of logging.
use crate::recording::SessionControl;
use kittx::{
    TransportConfig,
    datastore::*,
    transport::{Connector, SystemConnector, Transport},
};
use std::{
    collections::VecDeque,
    error::Error,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const RECENT_LIMIT: usize = 32;
const BYTE_LIMIT: usize = 4096;

struct Output {
    file: Option<File>,
    recent: VecDeque<String>,
    failed: bool,
}

pub(crate) struct SessionLog {
    pub(crate) path: PathBuf,
    debug: bool,
    began: Instant,
    output: Mutex<Output>,
}
impl SessionLog {
    pub(crate) fn open(recording: &Path, debug: bool) -> io::Result<Arc<Self>> {
        let mut path = recording.as_os_str().to_os_string();
        path.push(".log");
        let path = PathBuf::from(path);
        // Append to an existing diagnostic sidecar; never overwrite evidence.
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Arc::new(Self {
            path,
            debug,
            began: Instant::now(),
            output: Mutex::new(Output {
                file: Some(file),
                recent: VecDeque::new(),
                failed: false,
            }),
        }))
    }
    pub(crate) fn console(recording: &Path, debug: bool) -> Arc<Self> {
        Arc::new(Self {
            path: recording.with_extension("kittx.log"),
            debug,
            began: Instant::now(),
            output: Mutex::new(Output {
                file: None,
                recent: VecDeque::new(),
                failed: true,
            }),
        })
    }
    pub(crate) fn info(&self, message: &str) {
        self.emit("INFO", message);
    }
    pub(crate) fn debug(&self, message: &str) {
        if self.debug {
            self.emit("DEBUG", message);
        }
    }
    pub(crate) fn failure(&self, message: &str) {
        if !self.debug {
            let recent: Vec<_> = self
                .output
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .recent
                .iter()
                .cloned()
                .collect();
            for line in recent {
                self.emit("CONTEXT", &line);
            }
        }
        self.emit("ERROR", message);
    }
    fn emit(&self, level: &str, message: &str) {
        let utc_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let line = format!(
            "utc_ms={utc_ms} elapsed_ms={} {level} {message}",
            self.began.elapsed().as_millis()
        );
        let mut output = self.output.lock().unwrap_or_else(|e| e.into_inner());
        // A closed console or logging failure must not interrupt ECU/storage I/O.
        let _ = writeln!(io::stderr().lock(), "[KittxRecorder] {line}");
        if !output.failed
            && let Some(file) = &mut output.file
            && let Err(error) = writeln!(file, "{line}").and_then(|()| file.flush())
        {
            output.failed = true;
            let _ = writeln!(
                io::stderr().lock(),
                "[KittxRecorder] Cannot write {}: {error}; continuing with console diagnostics",
                self.path.display()
            );
        }
    }
    fn wire(&self, message: String) {
        let line = format!("elapsed_ms={} {message}", self.began.elapsed().as_millis());
        {
            let mut output = self.output.lock().unwrap_or_else(|e| e.into_inner());
            if output.recent.len() == RECENT_LIMIT {
                output.recent.pop_front();
            }
            output.recent.push_back(line.clone());
        }
        self.debug(&line);
    }
}

fn bytes(data: &[u8]) -> String {
    let shown = &data[..data.len().min(BYTE_LIMIT)];
    let hex = shown
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "len={} ascii=\"{}\" hex=[{hex}]{}",
        data.len(),
        shown.escape_ascii(),
        if shown.len() < data.len() {
            " (truncated)"
        } else {
            ""
        }
    )
}

pub(crate) fn describe_error(error: &(dyn Error + 'static)) -> String {
    let mut description = error.to_string();
    let mut current = Some(error);
    while let Some(cause) = current {
        if let Some(bridge) = cause.downcast_ref::<kittx::Error>() {
            description.push_str(&format!("; recovery={:?}", bridge.recovery));
            if !bridge.partial.is_empty() {
                description.push_str(&format!("; partial {}", bytes(&bridge.partial)));
            }
        }
        current = cause.source();
        if let Some(cause) = current {
            description.push_str(&format!("; caused by: {cause}"));
        }
    }
    description
}

pub(crate) struct DiagnosticConnector {
    pub(crate) log: Arc<SessionLog>,
}

#[cfg(target_os = "macos")]
fn macos_callout_config(
    config: &TransportConfig,
    exists: impl Fn(&Path) -> bool,
) -> TransportConfig {
    let mut resolved = config.clone();
    if let TransportConfig::Serial(serial) = &mut resolved
        && let Some(suffix) = serial
            .path
            .to_str()
            .and_then(|path| path.strip_prefix("/dev/tty."))
        && !suffix.is_empty()
        && !suffix.contains('/')
    {
        let callout = PathBuf::from(format!("/dev/cu.{suffix}"));
        if exists(&callout) {
            serial.path = callout;
        }
    }
    resolved
}

impl Connector for DiagnosticConnector {
    fn open(
        &mut self,
        config: &TransportConfig,
        timeout: Duration,
    ) -> kittx::Result<Box<dyn Transport>> {
        #[cfg(target_os = "macos")]
        let resolved = macos_callout_config(config, Path::exists);
        #[cfg(target_os = "macos")]
        let config = {
            if resolved != *config {
                self.log.info(&format!(
                    "Using macOS callout device: requested {config:?}; resolved {resolved:?}"
                ));
            }
            &resolved
        };
        self.log
            .info(&format!("Opening {config:?}; timeout={timeout:?}"));
        let inner = SystemConnector.open(config, timeout)?;
        self.log
            .info("Transport opened; synchronizing and initializing ELM327");
        Ok(Box::new(DiagnosticTransport {
            inner,
            log: self.log.clone(),
        }))
    }
}
struct DiagnosticTransport {
    inner: Box<dyn Transport>,
    log: Arc<SessionLog>,
}
impl Transport for DiagnosticTransport {
    fn read(&mut self, data: &mut [u8], timeout: Duration) -> io::Result<usize> {
        let began = Instant::now();
        let result = self.inner.read(data, timeout);
        let detail = match &result {
            Ok(n) => format!("RX {}", bytes(&data[..(*n).min(data.len())])),
            Err(error) => format!("RX error={:?}: {error}", error.kind()),
        };
        self.log.wire(format!(
            "{detail}; took={:?} timeout={timeout:?}",
            began.elapsed()
        ));
        result
    }
    fn write(&mut self, data: &[u8], timeout: Duration) -> io::Result<usize> {
        let began = Instant::now();
        let result = self.inner.write(data, timeout);
        let detail = match &result {
            Ok(n) => format!(
                "TX accepted={n}/{} {}",
                data.len(),
                bytes(&data[..(*n).min(data.len())])
            ),
            Err(error) => format!(
                "TX error={:?}: {error}; attempted {}",
                error.kind(),
                bytes(data)
            ),
        };
        self.log.wire(format!(
            "{detail}; took={:?} timeout={timeout:?}",
            began.elapsed()
        ));
        result
    }
    fn baud_rate(&self) -> Option<u32> {
        self.inner.baud_rate()
    }
    fn set_baud_rate(&mut self, baud: u32) -> io::Result<()> {
        let result = self.inner.set_baud_rate(baud);
        self.log
            .info(&format!("Set serial baud={baud}: {result:?}"));
        result
    }
}

pub(crate) struct DiagnosticSource<S> {
    pub(crate) inner: S,
    pub(crate) log: Arc<SessionLog>,
    pub(crate) control: Arc<SessionControl>,
}
impl<S: DataSource> DataSource for DiagnosticSource<S> {
    fn connect(&mut self, timeout: Duration) -> DataResult<SourceInfo> {
        let result = self.inner.connect(timeout);
        match &result {
            Ok(info) => self.log.info(&format!("Adapter ready: {info:?}")),
            Err(error) => {
                let detail = format!(
                    "Connection/initialization failed: {}",
                    describe_error(error)
                );
                self.log.failure(&detail);
                self.control.note_error(detail);
            }
        }
        result
    }
    fn read(&mut self, request: &ReadRequest, timeout: Duration) -> Observation {
        let start = format!("Read {request:?}; timeout={timeout:?}");
        if matches!(request, ReadRequest::Signal(_)) {
            self.log.debug(&start);
        } else {
            self.log.info(&start);
        }
        let began = Instant::now();
        let observation = self.inner.read(request, timeout);
        let mut errors: Vec<_> = observation.error.iter().cloned().collect();
        for response in &observation.responses {
            if let ObservationValue::Failure { kind, message } = &response.value {
                errors.push(format!("ECU {:?}: {kind}: {message}", response.sender));
            }
        }
        if !errors.is_empty() || observation.disconnected {
            let detail = format!(
                "Read {request:?} failed after {:?}: {}; protocol={:?}; recovery={:?}; disconnected={}; partial={}; request={}; response={}",
                began.elapsed(),
                errors.join("; "),
                observation.protocol,
                observation.recovery,
                observation.disconnected,
                observation.partial,
                bytes(observation.attempted_request.as_deref().unwrap_or_default()),
                bytes(observation.raw_response.as_deref().unwrap_or_default())
            );
            self.log.failure(&detail);
            self.control
                .note_error(format!("{request:?}: {}", errors.join("; ")));
        } else {
            self.log.debug(&format!(
                "Read {request:?} completed after {:?}: {:?}",
                began.elapsed(),
                observation.responses
            ));
        }
        observation
    }
    fn disconnect(&mut self, timeout: Duration) -> DataResult<()> {
        self.control.before_disconnect();
        let result = self.inner.disconnect(timeout);
        match &result {
            Ok(()) => self.log.info("Adapter disconnected"),
            Err(error) => self
                .log
                .failure(&format!("Disconnect failed: {}", describe_error(error))),
        }
        result
    }
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
