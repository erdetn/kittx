//! Recorder configuration and the available Mode 01 capture catalog.
use kittx::{
    TransportConfig,
    command::{CanId, Header},
    datastore::*,
    obd::{Pid, Sender},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct CaptureItem {
    pub id: u16,
    pub label: String,
    pub signal: Signal,
    pub decoded: bool,
}
/// Mode 01 capture choices, with names for documented PIDs and a raw fallback.
/// Bitmap queries and Service 02's freeze-frame DTC (02) are not live choices.
/// Undecoded PIDs retain complete raw ECU payloads; names do not imply support.
pub fn capture_catalog() -> Vec<CaptureItem> {
    let known = [
        (Pid::EngineLoad, "Engine load (%)"),
        (Pid::CoolantTemperature, "Coolant temperature (°C)"),
        (Pid::ManifoldPressure, "Manifold pressure (kPa)"),
        (Pid::EngineRpm, "Engine RPM"),
        (Pid::VehicleSpeed, "Vehicle speed (km/h)"),
        (Pid::IntakeTemperature, "Intake temperature (°C)"),
        (Pid::ThrottlePosition, "Throttle position (%)"),
    ];
    let mut items: Vec<_> = known
        .iter()
        .map(|&(pid, label)| CaptureItem {
            id: pid as u16,
            label: format!("{label} · 01:{:02X}", pid as u8),
            signal: Signal::Pid(pid),
            decoded: true,
        })
        .collect();
    items.push(CaptureItem {
        id: 256,
        label: "Adapter battery voltage (V) · ATRV".into(),
        signal: Signal::AdapterVoltage,
        decoded: true,
    });
    for pid in 1..=255u8 {
        if pid % 32 != 0 && pid != 0x02 && !known.iter().any(|(p, _)| *p as u8 == pid) {
            let name = crate::pid_catalog::description(pid)
                .unwrap_or_else(|| "Unassigned / unverified measurement".into());
            items.push(CaptureItem {
                id: u16::from(pid),
                label: format!("{name} · Mode 01 PID {pid:02X} · raw / undecoded"),
                signal: Signal::RawPid(pid),
                decoded: false,
            });
        }
    }
    items
}
#[derive(Clone, Debug)]
pub struct CaptureSettings {
    /// Application identity written to recording metadata.
    pub producer: String,
    pub transport: TransportConfig,
    pub selected: BTreeSet<u16>,
    pub dtcs: [bool; 3],
    pub profile: Option<DiagnosticProfile>,
    pub duration: Option<Duration>,
    pub sample_period: Duration,
    pub dtc_period: Duration,
    pub operation_timeout: Duration,
    pub vehicle: VehicleInfo,
}
impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            producer: "KittxRecorder".into(),
            transport: TransportConfig::tcp("127.0.0.1", 36000),
            selected: capture_catalog()
                .iter()
                .filter(|s| s.decoded)
                .map(|s| s.id)
                .collect(),
            dtcs: [true; 3],
            profile: None,
            duration: None,
            sample_period: Duration::from_secs(1),
            dtc_period: Duration::from_secs(60),
            operation_timeout: Duration::from_secs(10),
            vehicle: VehicleInfo::default(),
        }
    }
}
impl CaptureSettings {
    pub fn config(&self) -> Result<RecordingConfig, String> {
        self.transport.validate().map_err(|e| e.to_string())?;
        let catalog = capture_catalog();
        if self
            .selected
            .iter()
            .any(|id| !catalog.iter().any(|i| i.id == *id))
        {
            return Err("Unknown capture selection".into());
        }
        let kinds = [
            kittx::obd::DtcKind::Stored,
            kittx::obd::DtcKind::Pending,
            kittx::obd::DtcKind::Permanent,
        ]
        .into_iter()
        .zip(self.dtcs)
        .filter_map(|(k, on)| on.then_some(k))
        .collect::<Vec<_>>();
        if self.selected.is_empty() && kinds.is_empty() && self.profile.is_none() {
            return Err("Select at least one measurement or DTC category".into());
        }
        let config = RecordingConfig {
            signals: catalog
                .iter()
                .filter(|i| self.selected.contains(&i.id))
                .map(|i| SignalSelection::new(i.signal, self.sample_period))
                .collect(),
            dtcs: (!kinds.is_empty() || self.profile.is_some()).then_some(DtcSchedule {
                kinds,
                period: self.dtc_period,
                beginning: true,
                ending: true,
            }),
            profiles: self.profile.iter().cloned().collect(),
            duration: self.duration,
            operation_timeout: self.operation_timeout,
            vehicle: self.vehicle.clone(),
            producer: self.producer.clone(),
            producer_version: env!("CARGO_PKG_VERSION").into(),
            ..RecordingConfig::default()
        };
        config.validate().map_err(|e| e.to_string())?;
        Ok(config)
    }
}
/// Normalize the native save-dialog result. Existing files are never overwritten.
pub fn recording_path(path: &Path) -> Result<PathBuf, String> {
    if path.file_name().is_none() || path.is_dir() {
        return Err("Choose a recording filename".into());
    }
    let path = path.with_extension("kittx");
    if path.try_exists().map_err(|e| e.to_string())? {
        return Err(format!(
            "{} already exists; choose a new file",
            path.display()
        ));
    }
    Ok(path)
}
/// Positive whole-number interval. Units are explicit at the form field.
pub fn positive_duration(text: &str, milliseconds: bool) -> Result<Duration, String> {
    let value = text
        .trim()
        .parse::<u64>()
        .map_err(|_| "Enter a positive whole number".to_string())?;
    if value == 0 {
        return Err("Interval must be greater than zero".into());
    }
    let duration = if milliseconds {
        Duration::from_millis(value)
    } else {
        Duration::from_secs(value)
    };
    if duration.as_micros() > i64::MAX as u128 {
        return Err("Interval is too large".into());
    }
    Ok(duration)
}
/// Optional explicit addressing for a documented manufacturer read profile.
pub type ProfileAddress = (Option<Header>, Option<String>, Option<Sender>);
pub fn profile_address(header: &str, filter: &str, target: &str) -> Result<ProfileAddress, String> {
    let header = header.trim();
    let target = target.trim();
    let parse =
        |s: &str| u32::from_str_radix(s, 16).map_err(|_| "Address must be hexadecimal".to_string());
    let header = if header.is_empty() {
        None
    } else {
        Some(match header.len() {
            3 => Header::Can(CanId::Standard(parse(header)? as u16)),
            8 => Header::Can(CanId::Extended(parse(header)?)),
            6 => {
                let b = kittx::response::parse_hex(header).map_err(|e| e.to_string())?;
                Header::ThreeBytes([b[0], b[1], b[2]])
            }
            _ => return Err("Header needs 3, 6, or 8 hexadecimal digits".into()),
        })
    };
    if let Some(Header::Can(id)) = header {
        id.validate().map_err(|e| e.to_string())?;
    }
    let target = if target.is_empty() {
        None
    } else {
        let n = parse(target)?;
        Some(match target.len() {
            2 => Sender::Legacy(n as u8),
            3 if n <= 0x7ff => Sender::Can(n),
            8 if n <= 0x1fffffff => Sender::Can(n),
            _ => return Err("Target needs a valid 2-, 3-, or 8-digit ECU address".into()),
        })
    };
    let filter = (!filter.trim().is_empty()).then(|| filter.trim().to_uppercase());
    kittx::Command::CanReceive(filter.clone())
        .encode(b'\r')
        .map_err(|e| e.to_string())?;
    Ok((header, filter, target))
}
