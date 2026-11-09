#![cfg(feature = "datastore")]
use kittx::{
    Protocol,
    datastore::*,
    obd::{DtcKind, Pid, Sender},
};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

fn completed<T>(f: impl FnOnce(&mut dyn FnMut(DataResult<T>))) -> DataResult<T> {
    let mut result = None;
    f(&mut |r| result = Some(r));
    result.expect("completion callback")
}
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "kittx-data-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[derive(Default)]
struct State {
    elapsed: i64,
    jump: i64,
    reads: usize,
    connections: usize,
    disconnections: usize,
    fail_dtc: bool,
    disconnect: bool,
    vin: String,
    delay_us: i64,
    large: bool,
    invalid_number: bool,
    negative_peer: bool,
}
#[derive(Clone)]
struct FakeClock(Rc<RefCell<State>>);
impl Clock for FakeClock {
    fn now(&self) -> Timestamp {
        let s = self.0.borrow();
        Timestamp {
            utc_us: 1_700_000_000_000_000 + s.elapsed + s.jump,
            elapsed_us: s.elapsed,
        }
    }
    fn sleep(&self, d: Duration) {
        self.0.borrow_mut().elapsed += d.as_micros() as i64;
    }
}
struct FakeSource(Rc<RefCell<State>>);
impl DataSource for FakeSource {
    fn connect(&mut self, _: Duration) -> DataResult<SourceInfo> {
        self.0.borrow_mut().connections += 1;
        Ok(SourceInfo {
            transport: "test TCP".into(),
            adapter: "ELM327 v2.1".into(),
        })
    }
    fn disconnect(&mut self, _: Duration) -> DataResult<()> {
        self.0.borrow_mut().disconnections += 1;
        Ok(())
    }
    fn read(&mut self, request: &ReadRequest, timeout: Duration) -> Observation {
        let mut s = self.0.borrow_mut();
        s.reads += 1;
        let delay = s.delay_us.min(timeout.as_micros() as i64);
        s.elapsed += delay;
        if s.disconnect {
            s.disconnect = false;
            return Observation {
                error: Some("disconnected".into()),
                disconnected: true,
                partial: true,
                raw_response: Some(b"partial".to_vec()),
                ..Observation::default()
            };
        }
        if s.delay_us > timeout.as_micros() as i64 {
            return Observation {
                error: Some("timeout".into()),
                ..Observation::default()
            };
        }
        let value = match request {
            ReadRequest::Vin => ObservationValue::Vin(if s.vin.is_empty() {
                "WDB12345678901234".into()
            } else {
                s.vin.clone()
            }),
            ReadRequest::Signal(Signal::Pid(_) | Signal::AdapterVoltage) => {
                ObservationValue::Number(if s.invalid_number { f64::NAN } else { 42.0 })
            }
            ReadRequest::Signal(Signal::RawPid(p)) => ObservationValue::Raw(vec![0x41, *p, 7]),
            ReadRequest::Supported(_) => ObservationValue::Supported(u32::MAX),
            ReadRequest::Dtcs(DtcKind::Permanent) if s.fail_dtc => ObservationValue::Failure {
                kind: "unsupported".into(),
                message: "ECU rejects service 0A".into(),
            },
            ReadRequest::Dtcs(DtcKind::Pending) => ObservationValue::Dtcs(vec![]),
            ReadRequest::Dtcs(_) => ObservationValue::Dtcs(vec![DiagnosticCode {
                text: "P1337".into(),
                bytes: vec![0x13, 0x37],
                status: None,
            }]),
            ReadRequest::Diagnostic(_) => {
                ObservationValue::Raw(vec![0x59, 0x02, 0xFF, 0x12, 0x34, 0x56, 0x08])
            }
        };
        let second = if s.negative_peer && matches!(request, ReadRequest::Signal(_)) {
            ObservationValue::Failure {
                kind: "EcuNegative".into(),
                message: "negative peer response".into(),
            }
        } else {
            value.clone()
        };
        Observation {
            protocol: Some(Protocol::Can11_500),
            responses: vec![
                EcuObservation {
                    sender: Sender::Can(0x7E8),
                    value,
                },
                EcuObservation {
                    sender: Sender::Can(0x7E9),
                    value: second,
                },
            ],
            raw_response: Some(if s.large {
                vec![0x41; 4096]
            } else {
                b"test raw wire bytes\r>".to_vec()
            }),
            ..Observation::default()
        }
    }
}
fn config() -> RecordingConfig {
    RecordingConfig {
        signals: vec![SignalSelection::new(
            Signal::Pid(Pid::EngineRpm),
            Duration::from_secs(1),
        )],
        vehicle: VehicleInfo {
            make: Some(MetadataValue::caller("Mercedes-Benz")),
            model: Some(MetadataValue::caller("Test car")),
            ..VehicleInfo::default()
        },
        ..RecordingConfig::default()
    }
}
fn setup(config: RecordingConfig) -> (Recorder<FakeSource, FakeClock>, Rc<RefCell<State>>) {
    let state = Rc::new(RefCell::new(State::default()));
    (
        Recorder::with_clock(FakeSource(state.clone()), config, FakeClock(state.clone())).unwrap(),
        state,
    )
}

#[test]
fn selected_values_dtcs_metadata_paging_and_evidence() {
    let temp = Temp::new();
    let path = temp.path("trip.kittx");
    let mut cfg = config();
    cfg.signals.extend([
        SignalSelection::new(Signal::AdapterVoltage, Duration::from_secs(2)),
        SignalSelection::new(Signal::RawPid(0x62), Duration::from_secs(3)),
    ]);
    let (mut recorder, state) = setup(cfg);
    assert_eq!(recorder.state(), RecordingState::New);
    assert!(recorder.session_id().is_none());
    let thread = std::thread::current().id();
    completed(|cb| {
        recorder.start(&path, |r| {
            assert_eq!(thread, std::thread::current().id());
            cb(r)
        })
    })
    .unwrap();
    assert_eq!(recorder.state(), RecordingState::Recording);
    assert!(recorder.session_id().is_some());
    for i in 0..3 {
        completed(|cb| recorder.capture_signal(i, cb)).unwrap();
    }
    completed(|cb| recorder.scan_dtcs(ScanTrigger::Manual, cb)).unwrap();
    completed(|cb| recorder.set_period(0, Duration::from_millis(250), cb)).unwrap();
    let reader = RecordingReader::open(&path).unwrap();
    let info = reader.session().unwrap();
    assert_eq!(info.vin.as_deref(), Some("WDB12345678901234"));
    assert_eq!(info.vin_status, "verified");
    assert_eq!(info.manufacturer_coverage, "not_configured");
    assert!(info.started_utc_us.unwrap() >= info.created_utc_us);
    assert!(info.ended_utc_us.is_none());
    let fields = reader.vehicle_fields().unwrap();
    let make = fields.iter().find(|f| f.name == "make").unwrap();
    assert_eq!(make.value.as_deref(), Some("Mercedes-Benz"));
    assert_eq!(make.source.as_deref(), Some("caller"));
    assert_eq!(fields.len(), 6);
    let selections = reader.selections().unwrap();
    assert_eq!(selections[0].period_us, 250_000);
    assert_eq!(selections[0].capability, "supported");
    assert_eq!(selections[2].signal_id, "01:62:raw");
    let mut q = Query {
        limit: 1,
        ..Query::default()
    };
    let mut rows = Vec::new();
    loop {
        let page = reader.measurements(&q).unwrap();
        if page.is_empty() {
            break;
        }
        q.after_id = page[0].id;
        rows.extend(page);
    }
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[0].unit, "rpm");
    assert_eq!(rows[0].number, Some(42.0));
    assert_eq!(rows[4].bytes, Some(vec![0x41, 0x62, 7]));
    assert!(rows[0].utc_us > 0);
    let filtered = reader
        .measurements(&Query {
            ecu_id: Some(rows[0].ecu_id),
            signal_id: Some("01:0C".into()),
            from_elapsed_us: Some(0),
            to_elapsed_us: Some(10),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(filtered.len(), 1);
    let scans = reader.dtc_scans(&Query::default()).unwrap();
    assert!(
        scans
            .iter()
            .any(|s| s.category == "pending" && s.code_count == Some(0) && s.outcome == "ok")
    );
    let scan = scans.iter().find(|s| s.category == "stored").unwrap();
    let codes = reader.dtc_entries(scan.id, &Query::default()).unwrap();
    assert_eq!(codes[0].code, "P1337");
    assert_eq!(codes[0].bytes, [0x13, 0x37]);
    assert!(codes[0].interpretation.is_none());
    let evidence = reader.evidence(rows[0].acquisition_id).unwrap();
    assert_eq!(
        evidence.raw_response.as_deref(),
        Some(b"test raw wire bytes\r>".as_slice())
    );
    assert_eq!(evidence.scope, "ecu_transaction");
    assert!(!evidence.truncated);
    assert!(
        reader
            .acquisitions(&Query::default())
            .unwrap()
            .iter()
            .all(|r| r.completed.elapsed_us >= r.started.elapsed_us)
    );
    assert_eq!(reader.ecus(&Query::default()).unwrap().len(), 2);
    assert!(
        reader
            .events(&Query::default())
            .unwrap()
            .iter()
            .any(|e| e.kind == "period_changed")
    );
    drop(reader);
    let summary = completed(|cb| recorder.finish(cb)).unwrap();
    assert!(summary.ending_scan_complete);
    assert_eq!(summary.progress.acquired, summary.progress.committed);
    assert_eq!(state.borrow().disconnections, 1);
    assert_eq!(recorder.state(), RecordingState::Finished);
    let info = RecordingReader::open(&path).unwrap().session().unwrap();
    assert_eq!(info.status, "completed");
    assert!(info.finalized_utc_us.is_some());
    assert!(info.ending_scan_complete);
    assert!(!PathBuf::from(format!("{}-wal", path.display())).exists());
    assert_eq!(&std::fs::read(&path).unwrap()[..16], b"SQLite format 3\0");
}
#[test]
fn batching_snapshot_flush_and_interrupted_recovery() {
    let temp = Temp::new();
    let path = temp.path("live.kittx");
    let snapshot = temp.path("export.kittx");
    let mut cfg = config();
    cfg.dtcs = None;
    cfg.commit_policy = CommitPolicy::Batch {
        max_age: Duration::from_secs(1),
        max_acquisitions: 128,
        max_bytes: 1024 * 1024,
    };
    let (mut recorder, _) = setup(cfg);
    completed(|cb| recorder.start(&path, cb)).unwrap();
    let before = recorder.progress().committed;
    completed(|cb| recorder.capture_signal(0, cb)).unwrap();
    assert!(recorder.progress().acquired > before);
    assert_eq!(recorder.progress().committed, before);
    completed(|cb| recorder.snapshot(&snapshot, cb)).unwrap();
    assert_eq!(recorder.progress().committed, recorder.progress().acquired);
    let exported = RecordingReader::open(&snapshot).unwrap();
    let info = exported.session().unwrap();
    assert_eq!(info.status, "snapshot");
    assert!(info.exported_utc_us.is_some());
    assert!(info.ended_utc_us.is_none());
    assert_eq!(exported.measurements(&Query::default()).unwrap().len(), 2);
    completed(|cb| recorder.capture_signal(0, cb)).unwrap();
    drop(recorder);
    let recovered = RecordingReader::recover_interrupted(&path).unwrap();
    assert_eq!(recovered.session().unwrap().status, "interrupted");
    assert_eq!(recovered.measurements(&Query::default()).unwrap().len(), 2);
}
#[test]
fn hours_of_scheduling_are_bounded_and_dtc_scans_are_periodic() {
    let temp = Temp::new();
    let path = temp.path("hours.kittx");
    let mut cfg = config();
    cfg.duration = Some(Duration::from_secs(2 * 60 * 60));
    cfg.signals[0].period = Duration::from_secs(30);
    cfg.dtcs.as_mut().unwrap().period = Duration::from_secs(600);
    let (mut recorder, state) = setup(cfg);
    completed(|cb| recorder.start(&path, cb)).unwrap();
    completed(|cb| recorder.run(|_| RecordControl::Continue, cb)).unwrap();
    assert!(state.borrow().elapsed >= 7_200_000_000);
    let reader = RecordingReader::open(path).unwrap();
    let groups = reader.scan_groups(&Query::default()).unwrap();
    assert_eq!(groups.first().unwrap().trigger, "beginning");
    assert_eq!(groups.last().unwrap().trigger, "ending");
    assert!(groups.iter().any(|g| g.trigger == "periodic"));
    let rows = reader
        .measurements(&Query {
            limit: 1000,
            ..Query::default()
        })
        .unwrap();
    assert_eq!(rows.len(), 480);
    assert!(rows.windows(2).all(|w| w[0].elapsed_us <= w[1].elapsed_us));
    assert_eq!(reader.session().unwrap().status, "completed");
}
#[test]
fn failures_empty_results_raw_profiles_and_partial_ecus_are_distinct() {
    let temp = Temp::new();
    let path = temp.path("partial.kittx");
    let mut cfg = config();
    cfg.profiles.push(DiagnosticProfile {
        id: "verified-test-profile".into(),
        version: "1".into(),
        namespace: "vendor:test".into(),
        request: vec![0x19, 2, 0xFF],
        header: None,
        receive_filter: None,
        target: None,
    });
    let (mut recorder, state) = setup(cfg);
    state.borrow_mut().fail_dtc = true;
    completed(|cb| recorder.start(&path, cb)).unwrap();
    state.borrow_mut().negative_peer = true;
    completed(|cb| recorder.capture_signal(0, cb)).unwrap();
    let reader = RecordingReader::open(&path).unwrap();
    let scans = reader.dtc_scans(&Query::default()).unwrap();
    assert!(
        scans
            .iter()
            .any(|s| s.category == "permanent" && s.outcome == "failed" && s.code_count.is_none())
    );
    assert!(
        scans
            .iter()
            .any(|s| s.category == "pending" && s.outcome == "ok" && s.code_count == Some(0))
    );
    assert!(scans.iter().any(|s| s.outcome == "raw_only"
        && s.namespace == "vendor:test"
        && s.raw_payload.is_some()
        && s.code_count.is_none()));
    assert_eq!(reader.measurements(&Query::default()).unwrap().len(), 1);
    assert!(
        reader
            .acquisitions(&Query::default())
            .unwrap()
            .iter()
            .any(|a| a.kind == "measurement" && a.outcome == "partial")
    );
    assert_eq!(
        reader.scan_groups(&Query::default()).unwrap()[0].coverage,
        "partial"
    );
    drop(reader);
    completed(|cb| recorder.finish(cb)).unwrap();
}
#[test]
fn reconnect_clock_adjustment_vin_conflict_and_unavailable_ending_scan() {
    let temp = Temp::new();
    let path = temp.path("reconnect.kittx");
    let (mut recorder, state) = setup(config());
    completed(|cb| recorder.start(&path, cb)).unwrap();
    state.borrow_mut().disconnect = true;
    completed(|cb| recorder.capture_signal(0, cb)).unwrap();
    state.borrow_mut().jump = 3_000_000;
    completed(|cb| recorder.reconnect(cb)).unwrap();
    assert_eq!(state.borrow().connections, 2);
    completed(|cb| recorder.capture_signal(0, cb)).unwrap();
    let reader = RecordingReader::open(&path).unwrap();
    assert_eq!(reader.ecus(&Query::default()).unwrap().len(), 4);
    assert!(
        reader
            .events(&Query::default())
            .unwrap()
            .iter()
            .any(|e| e.kind == "clock_adjustment")
    );
    drop(reader);
    state.borrow_mut().vin = "1D4GP00R55B123456".into();
    assert!(completed(|cb| recorder.reconnect(cb)).is_err());
    assert_eq!(
        RecordingReader::open(&path)
            .unwrap()
            .session()
            .unwrap()
            .vin_status,
        "conflict"
    );
    assert!(completed(|cb| recorder.finish(cb)).is_err());
    let path = temp.path("ending.kittx");
    let (mut recorder, state) = setup(config());
    completed(|cb| recorder.start(&path, cb)).unwrap();
    state.borrow_mut().disconnect = true;
    completed(|cb| recorder.capture_signal(0, cb)).unwrap();
    let summary = completed(|cb| recorder.finish(cb)).unwrap();
    assert!(!summary.ending_scan_complete);
    assert_eq!(
        RecordingReader::open(path)
            .unwrap()
            .scan_groups(&Query::default())
            .unwrap()
            .last()
            .unwrap()
            .coverage,
        "partial"
    );
}
#[test]
fn input_limits_missing_vin_invalid_values_and_existing_files() {
    let temp = Temp::new();
    let path = temp.path("bad.kittx");
    let (mut recorder, state) = setup(config());
    state.borrow_mut().vin = "INVALID".into();
    assert!(completed(|cb| recorder.start(&path, cb)).is_err());
    let info = RecordingReader::open(&path).unwrap().session().unwrap();
    assert_eq!(info.status, "start_failed");
    assert!(info.vin.is_none());
    let (mut another, _) = setup(config());
    assert!(completed(|cb| another.start(&path, cb)).is_err());
    assert!(completed(|cb| another.start(temp.path("wrong.db"), cb)).is_err());
    let mut cfg = config();
    cfg.max_observation_bytes = 1024;
    let (mut recorder, state) = setup(cfg);
    let path = temp.path("limits.kittx");
    completed(|cb| recorder.start(&path, cb)).unwrap();
    state.borrow_mut().large = true;
    completed(|cb| recorder.capture_signal(0, cb)).unwrap();
    state.borrow_mut().large = false;
    state.borrow_mut().invalid_number = true;
    completed(|cb| recorder.capture_signal(0, cb)).unwrap();
    let reader = RecordingReader::open(&path).unwrap();
    assert!(reader.measurements(&Query::default()).unwrap().is_empty());
    let records = reader.acquisitions(&Query::default()).unwrap();
    let last = records.last().unwrap();
    assert_eq!(last.outcome, "failed");
    assert!(
        records
            .iter()
            .any(|a| reader.evidence(a.id).unwrap().truncated)
    );
    assert!(
        reader
            .measurements(&Query {
                limit: 1001,
                ..Query::default()
            })
            .is_err()
    );
    for bad in [
        RecordingConfig {
            operation_timeout: Duration::ZERO,
            ..config()
        },
        RecordingConfig {
            max_observation_bytes: 0,
            ..config()
        },
        RecordingConfig {
            signals: vec![SignalSelection::new(Signal::AdapterVoltage, Duration::ZERO)],
            ..config()
        },
    ] {
        assert!(bad.validate().is_err());
    }
    let mut duplicate = config();
    duplicate.signals.push(duplicate.signals[0].clone());
    assert!(duplicate.validate().is_err());
}
#[test]
fn deadline_overruns_stop_callback_and_target_filter() {
    let temp = Temp::new();
    let mut cfg = config();
    cfg.dtcs = None;
    cfg.duration = None;
    cfg.signals[0].target = Some(Sender::Can(0x7E8));
    cfg.signals[0].period = Duration::from_millis(1);
    let (mut recorder, state) = setup(cfg);
    let path = temp.path("overrun.kittx");
    completed(|cb| recorder.start(&path, cb)).unwrap();
    state.borrow_mut().delay_us = 10_000;
    let before = recorder.progress().acquired;
    completed(|cb| {
        recorder.run(
            |p| {
                if p.acquired >= before + 3 {
                    RecordControl::Stop
                } else {
                    RecordControl::Continue
                }
            },
            cb,
        )
    })
    .unwrap();
    let reader = RecordingReader::open(path).unwrap();
    assert_eq!(reader.measurements(&Query::default()).unwrap().len(), 3);
    assert!(
        reader
            .events(&Query::default())
            .unwrap()
            .iter()
            .any(|e| e.kind == "missed_schedule")
    );
}
#[test]
fn schema_identity_and_newer_versions_are_rejected_without_migration() {
    let temp = Temp::new();
    let path = temp.path("version.kittx");
    let (mut recorder, _) = setup(config());
    completed(|cb| recorder.start(&path, cb)).unwrap();
    drop(recorder);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.pragma_update(None, "user_version", 999).unwrap();
    drop(db);
    assert!(RecordingReader::open(&path).is_err());
    assert!(RecordingReader::recover_interrupted(&path).is_err());
}
#[test]
#[ignore = "subprocess crash fixture"]
fn crash_child() {
    let Some(path) = std::env::var_os("KITTX_CRASH_PATH") else {
        return;
    };
    let mut cfg = config();
    cfg.dtcs = None;
    cfg.commit_policy = CommitPolicy::Batch {
        max_age: Duration::from_secs(10),
        max_acquisitions: 128,
        max_bytes: 1024 * 1024,
    };
    let (mut recorder, _) = setup(cfg);
    completed(|cb| recorder.start(path, cb)).unwrap();
    completed(|cb| recorder.capture_signal(0, cb)).unwrap();
    completed(|cb| recorder.flush(cb)).unwrap();
    completed(|cb| recorder.capture_signal(0, cb)).unwrap();
    match std::env::var("KITTX_CRASH_PHASE").as_deref() {
        Ok("after_commit") => {
            completed(|cb| recorder.flush(cb)).unwrap();
        }
        Ok("during_transaction") => {
            let db =
                rusqlite::Connection::open(std::env::var_os("KITTX_CRASH_PATH").unwrap()).unwrap();
            db.execute_batch(
                "BEGIN IMMEDIATE; UPDATE session SET status='completed'; PRAGMA user_version=999;",
            )
            .unwrap();
            std::mem::forget(db);
        }
        _ => {}
    }
    std::process::exit(77);
}
#[test]
fn abrupt_process_exit_recovers_only_committed_observations() {
    let temp = Temp::new();
    let path = temp.path("crash.kittx");
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_child", "--ignored"])
        .env("KITTX_CRASH_PATH", &path)
        .output()
        .unwrap();
    assert_eq!(
        result.status.code(),
        Some(77),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let reader = RecordingReader::recover_interrupted(&path).unwrap();
    let info = reader.session().unwrap();
    assert_eq!(info.status, "interrupted");
    assert!(info.ended_utc_us.is_none());
    assert_eq!(reader.measurements(&Query::default()).unwrap().len(), 2);
    assert_eq!(info.committed_sequence, 3);
}

#[test]
fn storage_failure_rolls_back_whole_acquisition_and_stops_io() {
    let temp = Temp::new();
    let path = temp.path("write-failure.kittx");
    let mut cfg = config();
    cfg.dtcs = None;
    let (mut recorder, state) = setup(cfg);
    completed(|cb| recorder.start(&path, cb)).unwrap();
    let before = recorder.progress().committed;
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_measurement BEFORE INSERT ON measurement BEGIN SELECT RAISE(ABORT,'injected storage failure'); END;").unwrap();
    assert!(completed(|cb| recorder.capture_signal(0, cb)).is_err());
    assert_eq!(recorder.state(), RecordingState::Failed);
    assert_eq!(recorder.progress().committed, before);
    let reads = state.borrow().reads;
    assert!(completed(|cb| recorder.capture_signal(0, cb)).is_err());
    assert!(completed(|cb| recorder.flush(cb)).is_err());
    assert_eq!(state.borrow().reads, reads);
    let reader = RecordingReader::open(&path).unwrap();
    assert_eq!(reader.session().unwrap().committed_sequence, before as i64);
    assert!(reader.measurements(&Query::default()).unwrap().is_empty());
    assert_eq!(
        reader.acquisitions(&Query::default()).unwrap().len(),
        before as usize
    );
}
#[test]
fn busy_reader_prevents_false_standalone_finalization() {
    let temp = Temp::new();
    let path = temp.path("busy.kittx");
    let mut cfg = config();
    cfg.dtcs = None;
    let (mut recorder, _) = setup(cfg);
    completed(|cb| recorder.start(&path, cb)).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("BEGIN").unwrap();
    let _: i64 = db
        .query_row("SELECT count(*) FROM acquisition", [], |r| r.get(0))
        .unwrap();
    completed(|cb| recorder.capture_signal(0, cb)).unwrap();
    assert!(completed(|cb| recorder.finish(cb)).is_err());
    assert_ne!(
        RecordingReader::open(&path)
            .unwrap()
            .session()
            .unwrap()
            .status,
        "completed"
    );
    db.execute_batch("ROLLBACK").unwrap();
    drop(db);
    completed(|cb| recorder.finish(cb)).unwrap();
    assert_eq!(
        RecordingReader::open(path)
            .unwrap()
            .session()
            .unwrap()
            .status,
        "completed"
    );
}
#[test]
fn batch_limits_time_flush_before_slow_reads_and_invalid_clock() {
    let temp = Temp::new();
    let path = temp.path("bounded.kittx");
    let mut cfg = config();
    cfg.dtcs = None;
    cfg.operation_timeout = Duration::from_millis(10);
    cfg.commit_policy = CommitPolicy::Batch {
        max_age: Duration::from_secs(1),
        max_acquisitions: 2,
        max_bytes: 1024 * 1024,
    };
    let (mut recorder, state) = setup(cfg);
    completed(|cb| recorder.start(&path, cb)).unwrap();
    let before = recorder.progress().committed;
    completed(|cb| recorder.capture_signal(0, cb)).unwrap();
    assert_eq!(recorder.progress().committed, before);
    completed(|cb| recorder.capture_signal(0, cb)).unwrap();
    assert_eq!(recorder.progress().committed, before + 2);
    completed(|cb| recorder.capture_signal(0, cb)).unwrap();
    state.borrow_mut().elapsed = 995_000;
    state.borrow_mut().delay_us = 50_000;
    completed(|cb| recorder.capture_signal(0, cb)).unwrap();
    assert!(recorder.progress().committed >= before + 3);
    assert!(recorder.progress().failed_acquisitions >= 1);
    state.borrow_mut().elapsed = -1;
    assert!(completed(|cb| recorder.capture_signal(0, cb)).is_err());
}
#[test]
fn expired_shutdown_budget_records_every_unattempted_category() {
    let temp = Temp::new();
    let path = temp.path("shutdown.kittx");
    let mut cfg = config();
    cfg.shutdown_timeout = Duration::from_millis(1);
    let (mut recorder, state) = setup(cfg);
    completed(|cb| recorder.start(&path, cb)).unwrap();
    state.borrow_mut().delay_us = 10_000;
    let summary = completed(|cb| recorder.finish(cb)).unwrap();
    assert!(!summary.ending_scan_complete);
    let reader = RecordingReader::open(path).unwrap();
    let last = reader
        .scan_groups(&Query::default())
        .unwrap()
        .last()
        .unwrap()
        .id;
    let scans = reader.dtc_scans(&Query::default()).unwrap();
    let ending: Vec<_> = scans.iter().filter(|s| s.group_id == last).collect();
    assert_eq!(ending.len(), 3);
    assert!(
        ending
            .iter()
            .all(|s| s.outcome == "failed" && s.code_count.is_none())
    );
    assert!(ending.iter().any(|s| {
        s.error
            .as_ref()
            .is_some_and(|e| e.contains("budget expired"))
    }));
}

#[test]
fn crashes_during_transaction_and_after_commit_keep_consistent_prefix() {
    for (phase, expected) in [("during_transaction", 2), ("after_commit", 4)] {
        let temp = Temp::new();
        let path = temp.path("phase.kittx");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "crash_child", "--ignored"])
            .env("KITTX_CRASH_PATH", &path)
            .env("KITTX_CRASH_PHASE", phase)
            .output()
            .unwrap();
        assert_eq!(status.status.code(), Some(77));
        let reader = RecordingReader::recover_interrupted(path).unwrap();
        assert_eq!(reader.session().unwrap().status, "interrupted");
        assert_eq!(
            reader.measurements(&Query::default()).unwrap().len(),
            expected
        );
    }
}
