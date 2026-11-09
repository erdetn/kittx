use super::{DataResult, invalid};
use crate::{
    Protocol,
    command::Header,
    obd::{DtcKind, Pid, Sender},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const SCHEMA_VERSION: i32 = 1;
/// Project identifier `KITX`, not a registered SQLite application ID.
pub const APPLICATION_ID: i32 = 0x4b495458;
pub const FORMAT_ID: &str = "kittx.ecu-recording";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub utc_us: i64,
    pub elapsed_us: i64,
}
/// Inject a clock for simulations. Elapsed microseconds must never decrease.
pub trait Clock {
    fn now(&self) -> Timestamp;
    fn sleep(&self, duration: Duration);
}
pub struct SystemClock {
    anchor: Instant,
}
impl Default for SystemClock {
    fn default() -> Self {
        Self {
            anchor: Instant::now(),
        }
    }
}
impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp {
            utc_us: utc_now(),
            elapsed_us: self.anchor.elapsed().as_micros().min(i64::MAX as u128) as i64,
        }
    }
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}
pub(crate) fn utc_now() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_micros().min(i64::MAX as u128) as i64,
        Err(e) => -(e.duration().as_micros().min(i64::MAX as u128) as i64),
    }
}
pub(crate) fn micros(d: Duration) -> DataResult<i64> {
    let n =
        i64::try_from(d.as_micros()).map_err(|_| invalid("duration exceeds i64 microseconds"))?;
    if n == 0 {
        return Err(invalid("duration must be at least one microsecond"));
    }
    Ok(n)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataValue {
    pub value: String,
    pub source: String,
    pub source_version: Option<String>,
}
impl MetadataValue {
    pub fn caller(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            source: "caller".into(),
            source_version: None,
        }
    }
}
#[derive(Debug, Clone, Default)]
pub struct VehicleInfo {
    pub make: Option<MetadataValue>,
    pub model: Option<MetadataValue>,
    pub model_year: Option<MetadataValue>,
    pub engine: Option<MetadataValue>,
    pub powertrain: Option<MetadataValue>,
    pub name: Option<MetadataValue>,
}
impl VehicleInfo {
    pub(crate) fn fields(&self) -> [(&'static str, &Option<MetadataValue>); 6] {
        [
            ("make", &self.make),
            ("model", &self.model),
            ("model_year", &self.model_year),
            ("engine", &self.engine),
            ("powertrain", &self.powertrain),
            ("name", &self.name),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Pid(Pid),
    AdapterVoltage,
    RawPid(u8),
}
impl Signal {
    pub fn id(self) -> String {
        match self {
            Self::Pid(p) => format!("01:{:02X}", p as u8),
            Self::RawPid(p) => format!("01:{p:02X}:raw"),
            Self::AdapterVoltage => "adapter:voltage".into(),
        }
    }
    pub(crate) fn pid(self) -> Option<u8> {
        match self {
            Self::Pid(p) => Some(p as u8),
            Self::RawPid(p) => Some(p),
            _ => None,
        }
    }
    pub(crate) fn unit(self) -> &'static str {
        match self {
            Self::Pid(Pid::EngineRpm) => "rpm",
            Self::Pid(Pid::VehicleSpeed) => "km/h",
            Self::Pid(Pid::CoolantTemperature | Pid::IntakeTemperature) => "degC",
            Self::Pid(Pid::ManifoldPressure) => "kPa",
            Self::Pid(Pid::EngineLoad | Pid::ThrottlePosition) => "%",
            Self::AdapterVoltage => "V",
            Self::RawPid(_) => "raw",
        }
    }
}
#[derive(Debug, Clone)]
pub struct SignalSelection {
    pub signal: Signal,
    pub period: Duration,
    pub target: Option<Sender>,
}
impl SignalSelection {
    pub fn new(signal: Signal, period: Duration) -> Self {
        Self {
            signal,
            period,
            target: None,
        }
    }
}
#[derive(Debug, Clone)]
pub struct DiagnosticProfile {
    pub id: String,
    pub version: String,
    pub namespace: String,
    pub request: Vec<u8>,
    pub header: Option<Header>,
    pub receive_filter: Option<String>,
    pub target: Option<Sender>,
}
#[derive(Debug, Clone)]
pub struct DtcSchedule {
    pub kinds: Vec<DtcKind>,
    pub period: Duration,
    pub beginning: bool,
    pub ending: bool,
}
impl Default for DtcSchedule {
    fn default() -> Self {
        Self {
            kinds: vec![DtcKind::Stored, DtcKind::Pending, DtcKind::Permanent],
            period: Duration::from_secs(60),
            beginning: true,
            ending: true,
        }
    }
}
#[derive(Debug, Clone, Default)]
pub enum CommitPolicy {
    #[default]
    EachAcquisition,
    Batch {
        max_age: Duration,
        max_acquisitions: usize,
        max_bytes: usize,
    },
}
#[derive(Debug, Clone)]
pub struct RecordingConfig {
    pub vehicle: VehicleInfo,
    pub signals: Vec<SignalSelection>,
    pub dtcs: Option<DtcSchedule>,
    pub profiles: Vec<DiagnosticProfile>,
    pub duration: Option<Duration>,
    pub operation_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub commit_policy: CommitPolicy,
    pub producer: String,
    pub producer_version: String,
    pub previous_session: Option<String>,
    /// Maximum bytes in one admitted observation, including raw evidence.
    pub max_observation_bytes: usize,
}
impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            vehicle: VehicleInfo::default(),
            signals: Vec::new(),
            dtcs: Some(DtcSchedule::default()),
            profiles: Vec::new(),
            duration: None,
            operation_timeout: Duration::from_secs(10),
            shutdown_timeout: Duration::from_secs(15),
            commit_policy: CommitPolicy::default(),
            producer: "kittx".into(),
            producer_version: env!("CARGO_PKG_VERSION").into(),
            previous_session: None,
            max_observation_bytes: 1024 * 1024,
        }
    }
}
impl RecordingConfig {
    pub fn validate(&self) -> DataResult<()> {
        micros(self.operation_timeout)?;
        micros(self.shutdown_timeout)?;
        if let Some(d) = self.duration {
            micros(d)?;
        }
        if self.max_observation_bytes < 1024 || self.max_observation_bytes > 16 * 1024 * 1024 {
            return Err(invalid("observation limit must be 1024 bytes..16 MiB"));
        }
        if self.signals.len() > 256 || self.profiles.len() > 64 {
            return Err(invalid("too many selections/profiles"));
        }
        let mut seen = std::collections::HashSet::new();
        for s in &self.signals {
            micros(s.period)?;
            if !seen.insert((s.signal.id(), format!("{:?}", s.target))) {
                return Err(invalid("duplicate signal/target selection"));
            }
        }
        if let Some(d) = &self.dtcs {
            if d.kinds.is_empty() && self.profiles.is_empty() {
                return Err(invalid("empty DTC schedule: set dtcs=None instead"));
            }
            micros(d.period)?;
            let mut kinds = std::collections::HashSet::new();
            for k in &d.kinds {
                if !kinds.insert(k.service()) {
                    return Err(invalid("duplicate DTC category"));
                }
            }
        }
        let mut profiles = std::collections::HashSet::new();
        for p in &self.profiles {
            if p.id.is_empty()
                || p.version.is_empty()
                || p.namespace.is_empty()
                || p.id.len() > 4096
                || p.version.len() > 4096
                || p.namespace.len() > 4096
                || !profiles.insert(&p.id)
            {
                return Err(invalid("profiles need unique IDs, versions and namespaces"));
            }
            if p.request.first() == Some(&4) {
                return Err(invalid(
                    "service 04 clears diagnostics and is not a recording read profile",
                ));
            }
            crate::Command::Vehicle {
                bytes: p.request.clone(),
                responses: None,
            }
            .encode(b'\r')?;
            if let Some(h) = p.header {
                crate::Command::Header(h).encode(b'\r')?;
            }
            crate::Command::CanReceive(p.receive_filter.clone()).encode(b'\r')?;
        }
        if !self.profiles.is_empty() && self.dtcs.is_none() {
            return Err(invalid("profiles require a DTC schedule"));
        }
        if let CommitPolicy::Batch {
            max_age,
            max_acquisitions,
            max_bytes,
        } = self.commit_policy
        {
            micros(max_age)?;
            if max_acquisitions == 0
                || max_acquisitions > 4096
                || max_bytes < self.max_observation_bytes
                || max_bytes > 64 * 1024 * 1024
            {
                return Err(invalid(
                    "batch limits: 1..4096 acquisitions, observation limit <= bytes <= 64 MiB",
                ));
            }
        }
        for (_, v) in self.vehicle.fields() {
            if let Some(v) = v {
                if v.value.len() > 4096
                    || v.source.is_empty()
                    || v.source.len() > 4096
                    || v.source_version.as_ref().is_some_and(|s| s.len() > 4096)
                {
                    return Err(invalid("invalid vehicle provenance/value"));
                }
            }
        }
        if self.producer.len() > 4096
            || self.producer_version.len() > 4096
            || self
                .previous_session
                .as_ref()
                .is_some_and(|s| s.len() > 4096)
        {
            return Err(invalid("metadata too long"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanTrigger {
    Beginning,
    Periodic,
    Ending,
    Manual,
}
impl ScanTrigger {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Beginning => "beginning",
            Self::Periodic => "periodic",
            Self::Ending => "ending",
            Self::Manual => "manual",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingState {
    New,
    Preparing,
    Recording,
    Failed,
    Finished,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordControl {
    Continue,
    Stop,
}
#[derive(Debug, Clone, Default)]
pub struct RecordingProgress {
    pub acquired: u64,
    pub committed: u64,
    pub failed_acquisitions: u64,
}
#[derive(Debug, Clone)]
pub struct RecordingSummary {
    pub session_id: String,
    pub path: std::path::PathBuf,
    pub progress: RecordingProgress,
    pub ending_scan_complete: bool,
}
#[derive(Debug, Clone)]
pub enum ReadRequest {
    Vin,
    Signal(Signal),
    Dtcs(DtcKind),
    Supported(u8),
    Diagnostic(DiagnosticProfile),
}
impl ReadRequest {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Vin => "vin",
            Self::Signal(_) => "measurement",
            Self::Dtcs(_) => "dtc",
            Self::Supported(_) => "capability",
            Self::Diagnostic(_) => "diagnostic_raw",
        }
    }
    pub(crate) fn description(&self) -> String {
        match self {
            Self::Vin => "09:02".into(),
            Self::Signal(s) => s.id(),
            Self::Dtcs(k) => format!("{:02X}", k.service()),
            Self::Supported(b) => format!("01:{b:02X}:support"),
            Self::Diagnostic(p) => format!("{}@{}", p.id, p.version),
        }
    }
}
#[derive(Debug, Clone)]
pub struct DiagnosticCode {
    pub text: String,
    pub bytes: Vec<u8>,
    pub status: Option<Vec<u8>>,
}
#[derive(Debug, Clone)]
pub enum ObservationValue {
    Vin(String),
    Number(f64),
    Raw(Vec<u8>),
    Dtcs(Vec<DiagnosticCode>),
    Supported(u32),
    Failure { kind: String, message: String },
}
#[derive(Debug, Clone)]
pub struct EcuObservation {
    pub sender: Sender,
    pub value: ObservationValue,
}
#[derive(Debug, Clone, Default)]
pub struct Observation {
    pub protocol: Option<Protocol>,
    pub responses: Vec<EcuObservation>,
    pub raw_response: Option<Vec<u8>>,
    /// Encoded intended request; does not prove every byte reached the adapter.
    pub attempted_request: Option<Vec<u8>>,
    pub context: Option<String>,
    pub partial: bool,
    pub error: Option<String>,
    pub recovery: Option<String>,
    pub disconnected: bool,
}
impl Observation {
    pub(crate) fn failed(&self) -> bool {
        self.error.is_some()
            || self
                .responses
                .iter()
                .any(|e| matches!(e.value, ObservationValue::Failure { .. }))
    }
    pub(crate) fn bytes(&self) -> usize {
        let mut n = self.raw_response.as_ref().map_or(0, Vec::len)
            + self.error.as_ref().map_or(0, String::len)
            + self.recovery.as_ref().map_or(0, String::len);
        n = n
            .saturating_add(self.attempted_request.as_ref().map_or(0, Vec::len))
            .saturating_add(self.context.as_ref().map_or(0, String::len));
        for e in &self.responses {
            n = n.saturating_add(
                128 + match &e.value {
                    ObservationValue::Vin(v) => v.len(),
                    ObservationValue::Raw(v) => v.len(),
                    ObservationValue::Dtcs(v) => v
                        .iter()
                        .map(|c| {
                            c.text.len()
                                + c.bytes.len()
                                + c.status.as_ref().map_or(0, Vec::len)
                                + 64
                        })
                        .sum(),
                    ObservationValue::Failure { kind, message } => kind.len() + message.len(),
                    _ => 8,
                },
            );
        }
        n
    }
}
#[derive(Debug, Clone)]
pub struct SourceInfo {
    pub transport: String,
    pub adapter: String,
}
/// Sender keys are scoped by connection segment and protocol in the database.
pub(crate) fn sender_key(sender: Sender) -> String {
    match sender {
        Sender::Can(id) => format!("can:{id:08X}"),
        Sender::Legacy(id) => format!("legacy:{id:02X}"),
        Sender::Unknown => "unknown".into(),
    }
}
