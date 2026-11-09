use kittx::{
    Protocol, TransportConfig,
    datastore::*,
    obd::{DtcKind, Sender},
};
use kittx_recorder::{model::*, recording::*};
use kittx_viewer::{document::Document, model::Viewport};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "kittx-recorder-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self) -> PathBuf {
        self.0.join("trip.kittx")
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[derive(Clone)]
struct FakeClock(Rc<Cell<i64>>);
impl Clock for FakeClock {
    fn now(&self) -> Timestamp {
        Timestamp {
            utc_us: 1_700_000_000_000_000 + self.0.get(),
            elapsed_us: self.0.get(),
        }
    }
    fn sleep(&self, d: Duration) {
        self.0.set(self.0.get() + d.as_micros() as i64)
    }
}
#[derive(Default)]
struct Stats {
    signals: usize,
    disconnects: usize,
    connects: usize,
    fail_connect: bool,
    fail_dtc: bool,
    stop_after: Option<usize>,
    disconnect_after: Option<usize>,
}
struct Source {
    clock: FakeClock,
    stats: Rc<RefCell<Stats>>,
    control: Arc<SessionControl>,
}
impl DataSource for Source {
    fn connect(&mut self, _: Duration) -> DataResult<SourceInfo> {
        let mut s = self.stats.borrow_mut();
        s.connects += 1;
        if s.fail_connect {
            return Err(DataError::State("scripted connection failure".into()));
        }
        Ok(SourceInfo {
            transport: "fake raw TCP".into(),
            adapter: "ELM327 v2.1".into(),
        })
    }
    fn disconnect(&mut self, _: Duration) -> DataResult<()> {
        self.stats.borrow_mut().disconnects += 1;
        Ok(())
    }
    fn read(&mut self, request: &ReadRequest, _: Duration) -> Observation {
        self.clock.sleep(Duration::from_millis(1));
        let mut s = self.stats.borrow_mut();
        let value = match request {
            ReadRequest::Vin => ObservationValue::Vin("WDB12345678901234".into()),
            ReadRequest::Supported(_) => ObservationValue::Supported(u32::MAX),
            ReadRequest::Signal(signal) => {
                s.signals += 1;
                if s.disconnect_after == Some(s.signals) {
                    return Observation {
                        error: Some("scripted adapter disconnect".into()),
                        disconnected: true,
                        raw_response: Some(b"partial disconnect evidence".to_vec()),
                        ..Default::default()
                    };
                }
                if s.stop_after.is_some_and(|n| s.signals >= n) {
                    self.control.request_stop();
                }
                match signal {
                    Signal::RawPid(pid) => ObservationValue::Raw(vec![0x41, *pid, 0x80]),
                    _ => ObservationValue::Number(42.),
                }
            }
            ReadRequest::Dtcs(DtcKind::Permanent) if s.fail_dtc => ObservationValue::Failure {
                kind: "unsupported".into(),
                message: "scripted unsupported permanent DTCs".into(),
            },
            ReadRequest::Dtcs(DtcKind::Pending) => ObservationValue::Dtcs(vec![]),
            ReadRequest::Dtcs(_) => ObservationValue::Dtcs(vec![DiagnosticCode {
                text: "P1337".into(),
                bytes: vec![0x13, 0x37],
                status: None,
            }]),
            ReadRequest::Diagnostic(_) => {
                ObservationValue::Raw(vec![0x59, 2, 0xff, 0x12, 0x34, 0x56, 8])
            }
        };
        Observation {
            protocol: Some(Protocol::Can11_500),
            responses: vec![EcuObservation {
                sender: Sender::Can(0x7e8),
                value,
            }],
            raw_response: Some(b"scripted evidence\r>".to_vec()),
            ..Default::default()
        }
    }
}
fn setup(path: PathBuf) -> (Source, FakeClock, Rc<RefCell<Stats>>, Arc<SessionControl>) {
    let clock = FakeClock(Rc::new(Cell::new(0)));
    let stats = Rc::new(RefCell::new(Stats::default()));
    let control = Arc::new(SessionControl::new(path));
    (
        Source {
            clock: clock.clone(),
            stats: stats.clone(),
            control: control.clone(),
        },
        clock,
        stats,
        control,
    )
}
#[test]
fn timed_capture_is_readable_and_plottable_by_viewer() {
    let temp = Temp::new();
    let path = temp.path();
    let (source, clock, stats, control) = setup(path.clone());
    stats.borrow_mut().fail_dtc = true;
    let settings = CaptureSettings {
        selected: [0x0c, 0x61, 256].into_iter().collect(),
        vehicle: VehicleInfo {
            make: Some(MetadataValue::caller("Mercedes-Benz")),
            model: Some(MetadataValue::caller("C-Class")),
            ..Default::default()
        },
        duration: Some(Duration::from_secs(2)),
        sample_period: Duration::from_millis(400),
        dtc_period: Duration::from_millis(700),
        profile: Some(DiagnosticProfile {
            id: "vendor-read".into(),
            version: "1".into(),
            namespace: "test-vendor".into(),
            request: vec![0x19, 2, 0xff],
            header: None,
            receive_filter: None,
            target: None,
        }),
        ..Default::default()
    };
    let summary = run_session(source, clock, settings, path.clone(), &control).unwrap();
    assert!(!summary.ending_scan_complete);
    assert!(summary.progress.committed > 10);
    assert!(summary.progress.failed_acquisitions > 0);
    assert_eq!(stats.borrow().disconnects, 1);
    assert_eq!(control.snapshot().phase, Phase::Completed);
    assert_eq!(control.snapshot().vin.as_deref(), Some("WDB12345678901234"));
    assert!(control.snapshot().summary.is_some());
    let reader = RecordingReader::open(&path).unwrap();
    assert_eq!(reader.session().unwrap().status, "completed");
    let scans = reader
        .dtc_scans(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    for category in ["stored", "pending", "permanent", "manufacturer_raw"] {
        assert!(scans.iter().any(|s| s.category == category));
    }
    assert!(
        reader
            .scan_groups(&Query::default())
            .unwrap()
            .iter()
            .any(|g| g.trigger == "periodic")
    );
    assert!(
        reader
            .events(&Query::default())
            .unwrap()
            .iter()
            .all(|e| !e.kind.contains("clear"))
    );
    let doc = Document::open(path).unwrap();
    for (field, value) in [("make", "Mercedes-Benz"), ("model", "C-Class")] {
        let metadata = doc
            .overview
            .vehicle
            .iter()
            .find(|item| item.name == field)
            .unwrap();
        assert_eq!(metadata.value.as_deref(), Some(value));
        assert_eq!(metadata.source.as_deref(), Some("caller"));
    }
    let indices: Vec<_> = doc
        .overview
        .series
        .iter()
        .enumerate()
        .filter(|(_, s)| s.numeric)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(indices.len(), 3); // RPM, adapter voltage, and viewer-decoded PID 61.
    let raw = doc.overview.series.iter().position(|s| !s.numeric).unwrap();
    assert!(doc.raw_sample(raw).unwrap().contains("41, 61, 80"));
    let frame = doc
        .frame(doc.full(), &indices, 800, Default::default())
        .unwrap();
    assert!(frame.samples.iter().all(|(_, p)| !p.is_empty()));
    let torque = doc
        .overview
        .series
        .iter()
        .position(|s| s.signal == "01:61:decoded:TQ_DD")
        .unwrap();
    assert!(
        frame
            .samples
            .iter()
            .find(|(index, _)| *index == torque)
            .unwrap()
            .1
            .iter()
            .all(|p| p.value == 3.)
    );
    assert!(!frame.scans.is_empty());
}
#[test]
fn manual_stop_finalizes_and_keeps_selected_measurements_only() {
    let temp = Temp::new();
    let (source, clock, stats, control) = setup(temp.path());
    stats.borrow_mut().stop_after = Some(3);
    let settings = CaptureSettings {
        selected: [0x0d].into_iter().collect(),
        duration: None,
        ..Default::default()
    };
    let summary = run_session(source, clock, settings, temp.path(), &control).unwrap();
    assert!(summary.ending_scan_complete);
    assert_eq!(stats.borrow().signals, 3);
    let r = RecordingReader::open(temp.path()).unwrap();
    assert_eq!(r.session().unwrap().status, "completed");
    assert!(
        r.measurements(&Query::default())
            .unwrap()
            .iter()
            .all(|m| m.signal_id == "01:0D")
    );
    control.request_stop();
    assert_eq!(control.snapshot().phase, Phase::Completed);
}
#[test]
fn dtc_only_and_signal_only_sessions_are_supported() {
    for dtc_only in [true, false] {
        let temp = Temp::new();
        let (source, clock, _, control) = setup(temp.path());
        let settings = CaptureSettings {
            duration: Some(Duration::from_millis(50)),
            selected: if dtc_only {
                Default::default()
            } else {
                [0x0c].into_iter().collect()
            },
            dtcs: if dtc_only { [true; 3] } else { [false; 3] },
            ..Default::default()
        };
        run_session(source, clock, settings, temp.path(), &control).unwrap();
        let r = RecordingReader::open(temp.path()).unwrap();
        assert_eq!(
            r.measurements(&Query::default()).unwrap().is_empty(),
            dtc_only
        );
        assert_eq!(
            r.dtc_scans(&Query::default()).unwrap().is_empty(),
            !dtc_only
        );
        assert!(
            Document::open(temp.path())
                .unwrap()
                .frame(Viewport::new(0, 1_000_000), &[], 10, Default::default())
                .is_ok()
        );
    }
}
#[test]
fn failed_start_retains_compatible_file_and_reports_failure() {
    let temp = Temp::new();
    let (source, clock, stats, control) = setup(temp.path());
    stats.borrow_mut().fail_connect = true;
    assert!(
        run_session(
            source,
            clock,
            CaptureSettings::default(),
            temp.path(),
            &control
        )
        .is_err()
    );
    assert_eq!(control.snapshot().phase, Phase::Failed);
    assert!(control.snapshot().message.contains("scripted"));
    let doc = Document::open(temp.path()).unwrap();
    assert_eq!(doc.overview.session.status, "start_failed");
}
#[test]
fn catalog_paths_intervals_and_validation() {
    let catalog = capture_catalog();
    assert_eq!(catalog.iter().filter(|s| s.decoded).count(), 8);
    let mut ids = std::collections::BTreeSet::new();
    for item in &catalog {
        assert!(ids.insert(item.id));
        if item.id != 256 {
            assert_ne!(item.id % 32, 0);
        }
    }
    assert_eq!(catalog.len(), 248);
    let mut settings = CaptureSettings {
        selected: ids,
        ..Default::default()
    };
    assert_eq!(settings.config().unwrap().signals.len(), 248);
    settings.selected.insert(999);
    assert!(settings.config().is_err());
    settings.selected.clear();
    settings.dtcs = [false; 3];
    assert!(settings.config().is_err());
    let temp = Temp::new();
    assert_eq!(
        recording_path(&temp.0.join("trip.SQLITE")).unwrap(),
        temp.path()
    );
    std::fs::write(temp.path(), b"keep").unwrap();
    assert!(RecordingJob::spawn(CaptureSettings::default(), temp.path()).is_err());
    assert_eq!(std::fs::read(temp.path()).unwrap(), b"keep");
    assert!(recording_path(&temp.0).is_err());
    assert_eq!(
        positive_duration("100", true).unwrap(),
        Duration::from_millis(100)
    );
    assert_eq!(
        positive_duration("2", false).unwrap(),
        Duration::from_secs(2)
    );
    for value in ["0", "-1", "abc", "18446744073709551615"] {
        assert!(positive_duration(value, false).is_err());
    }
    settings = CaptureSettings::default();
    settings.transport = TransportConfig::tcp("", 0);
    assert!(settings.config().is_err());
}
#[test]
fn manufacturer_profile_addressing_and_destructive_service_rejection() {
    let (h, filter, target) = profile_address("7E0", "7E8", "7E8").unwrap();
    assert!(h.is_some());
    assert_eq!(filter.as_deref(), Some("7E8"));
    assert_eq!(target, Some(Sender::Can(0x7e8)));
    assert!(profile_address("81F110", "", "10").unwrap().0.is_some());
    assert!(
        profile_address("18DA10F1", "18DAF110", "18DAF110")
            .unwrap()
            .0
            .is_some()
    );
    assert!(profile_address("12", "", "").is_err());
    assert!(profile_address("", "", "FFFFFFFF").is_err());
    let settings = CaptureSettings {
        profile: Some(DiagnosticProfile {
            id: "invalid-clear".into(),
            version: "1".into(),
            namespace: "test".into(),
            request: vec![4],
            header: None,
            receive_filter: None,
            target: None,
        }),
        ..Default::default()
    };
    assert!(settings.config().is_err());
    let c = SessionControl::new(PathBuf::from("pending.kittx"));
    assert_eq!(c.snapshot().phase, Phase::Preparing);
    c.request_stop();
    assert_eq!(c.snapshot().phase, Phase::Stopping);
}
#[test]
fn worker_failures_are_terminal_and_reap_is_nonblocking() {
    let temp = Temp::new();
    let path = temp.0.join("missing-parent").join("trip.kittx");
    let mut job = RecordingJob::spawn(CaptureSettings::default(), path).unwrap();
    job.request_stop();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !job.is_finished() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(job.reap());
    assert!(job.reap());
    assert_eq!(job.snapshot().phase, Phase::Failed);
    assert!(!job.snapshot().message.is_empty());
    job.request_stop();
    assert_eq!(job.snapshot().phase, Phase::Failed);
}

#[test]
fn lost_connection_finalizes_committed_prefix_and_surfaces_failure() {
    let temp = Temp::new();
    let (source, clock, stats, control) = setup(temp.path());
    stats.borrow_mut().disconnect_after = Some(2);
    let settings = CaptureSettings {
        selected: [0x0c].into_iter().collect(),
        ..Default::default()
    };
    let error = run_session(source, clock, settings, temp.path(), &control).unwrap_err();
    assert!(error.contains("Capture failed"));
    let status = control.snapshot();
    assert_eq!(status.phase, Phase::Failed);
    assert!(status.summary.is_some());
    assert!(status.progress.committed > 0);
    let doc = Document::open(temp.path()).unwrap();
    assert_eq!(doc.overview.session.status, "completed");
    assert!(!doc.overview.session.ending_scan_complete);
    assert!(doc.overview.series.iter().any(|s| s.numeric));
    assert_eq!(stats.borrow().connects, 1);
}
