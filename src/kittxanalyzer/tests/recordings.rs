#[path = "../../../tests/common/viewer_fixture.rs"]
mod fixture;
use fixture::Fixture;
use kittx_analyzer::model::{Recording, Selection, signal_label};

#[test]
fn catalog_retains_ecu_category_namespace_profile_and_read_outcomes() {
    let f = Fixture::new();
    for id in 1..=9 {
        f.sample(id, id as f64);
    }
    for id in 1..=4 {
        f.scan(id, "ok", Some(1));
    }
    f.scan(5, "raw_only", None);
    f.scan(6, "failed", None);
    f.scan(7, "ok", Some(0));
    f.scan(8, "ok", Some(1));
    f.scan(9, "ok", Some(1));
    f.db.execute_batch("UPDATE session SET committed_sequence=8,started_utc_us=created_utc_us;
        UPDATE dtc_scan_result SET category='pending',ecu_id=2 WHERE id=3;
        UPDATE dtc_scan_result SET category='permanent' WHERE id=4;
        UPDATE dtc_entry SET code='P1999' WHERE id=4;
        INSERT INTO diagnostic_profile VALUES('vendor-read','1','vendor',X'1902',NULL,NULL,NULL);
        UPDATE dtc_scan_result SET category='manufacturer',namespace='vendor',profile_id='vendor-read',raw_payload=X'DEADBEEF' WHERE id=5;
        UPDATE dtc_scan_result SET ecu_id=NULL WHERE id=6;
        DELETE FROM dtc_entry WHERE id=8;
        UPDATE dtc_entry SET code='P0999' WHERE id=9;
        INSERT INTO measurement VALUES(100,1,1,'raw',1000,1000,'blob',NULL,X'112233','bytes');
        INSERT INTO measurement VALUES(101,1,2,'speed',1000,1000,'number',44,NULL,'km/h');
        INSERT INTO vehicle_field VALUES('make','Toyota','caller',NULL,0);").unwrap();
    let before = std::fs::read(&f.path).unwrap();
    let recording = Recording::open(f.path.clone()).unwrap();
    assert_eq!(recording.path, f.path);
    assert_eq!(recording.overview.session.committed_sequence, 8);
    assert_eq!(recording.overview.series.len(), 3);
    assert_eq!(
        recording
            .overview
            .series
            .iter()
            .filter(|s| s.numeric)
            .count(),
        2
    );
    assert_eq!(
        recording
            .overview
            .series
            .iter()
            .find(|s| !s.numeric)
            .unwrap()
            .signal,
        "raw"
    );
    assert_eq!(
        recording.overview.vehicle[0].value.as_deref(),
        Some("Toyota")
    );
    let stored = recording
        .dtcs
        .iter()
        .find(|d| d.category == "stored" && d.code.as_deref() == Some("P0123"))
        .unwrap();
    assert_eq!(stored.observations, 2);
    assert_eq!((stored.first_us, stored.last_us), (1000, 2000));
    assert_eq!(stored.ecu_id, Some(1));
    assert_eq!(stored.sender.as_deref(), Some("7E8"));
    assert_eq!(stored.namespace, "OBD");
    assert_eq!(stored.reported_codes, Some(1));
    assert_eq!(stored.outcome, "ok");
    assert!(stored.profile_id.is_none());
    assert!(
        stored
            .label()
            .contains("P0123 · Stored · OBD · ECU 1 (7E8)")
    );
    let pending = recording
        .dtcs
        .iter()
        .find(|d| d.category == "pending")
        .unwrap();
    assert_eq!(pending.ecu_id, Some(2));
    assert!(pending.label().contains("Pending"));
    assert!(
        recording
            .dtcs
            .iter()
            .any(|d| d.label().contains("P1999 · Permanent"))
    );
    let raw = recording
        .dtcs
        .iter()
        .find(|d| d.outcome == "raw_only")
        .unwrap();
    assert_eq!(raw.profile_id.as_deref(), Some("vendor-read"));
    assert!(
        raw.label()
            .contains("Raw diagnostic replies · manufacturer · vendor")
    );
    assert!(raw.label().contains("profile vendor-read"));
    let failed = recording
        .dtcs
        .iter()
        .find(|d| d.outcome == "failed")
        .unwrap();
    assert_eq!(failed.ecu_id, None);
    assert_eq!(failed.sender, None);
    assert!(
        failed
            .label()
            .contains("Failed DTC reads · Stored · OBD · ECU -")
    );
    assert!(
        recording
            .dtcs
            .iter()
            .any(|d| d.label().contains("Empty DTC reads"))
    );
    assert!(
        recording
            .dtcs
            .iter()
            .any(|d| d.label().contains("No decoded DTC entries"))
    );
    assert!(
        !recording
            .dtcs
            .iter()
            .any(|d| d.code.as_deref() == Some("P0999"))
    );
    let preview = recording
        .selection_preview(&Selection::all(&recording))
        .unwrap();
    assert!(preview.contains("raw / undecoded"));
    assert!(preview.contains("profile vendor-read"));
    assert!(!preview.contains("11, 22, 33"));
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
}

#[test]
fn preview_uses_only_selected_data_and_never_claims_analysis() {
    let f = Fixture::new();
    f.sample(1, 42.);
    f.sample(2, 43.);
    f.scan(1, "ok", Some(1));
    f.scan(2, "raw_only", None);
    f.db.execute_batch(
        "UPDATE session SET started_utc_us=created_utc_us;
        UPDATE dtc_scan_result SET namespace='vendor',raw_payload=X'DEADBEEF' WHERE id=2;",
    )
    .unwrap();
    let before = std::fs::read(&f.path).unwrap();
    let recording = Recording::open(f.path.clone()).unwrap();
    let mut selected = Selection::default();
    assert!(recording.selection_preview(&selected).is_err());
    selected.series.insert(0);
    let measurements = recording.selection_preview(&selected).unwrap();
    assert!(measurements.contains("no analysis has run"));
    assert!(measurements.contains("Measurements (1)"));
    assert!(measurements.contains("2 samples"));
    assert!(measurements.contains("DTC items (0)"));
    assert!(!measurements.contains("P0123"));
    selected.series.clear();
    selected.dtcs.insert(
        recording
            .dtcs
            .iter()
            .position(|d| d.code.is_some())
            .unwrap(),
    );
    let dtc_only = recording.selection_preview(&selected).unwrap();
    assert!(dtc_only.contains("Measurements (0)"));
    assert!(dtc_only.contains("P0123"));
    assert!(dtc_only.contains("observed 0.001000–0.001000 s"));
    assert!(!dtc_only.contains("Raw diagnostic replies"));
    let all = Selection::all(&recording);
    assert_eq!(all.series, [0].into_iter().collect());
    assert_eq!(all.dtcs.len(), 2);
    let preview = recording.selection_preview(&all).unwrap();
    assert!(preview.contains("Raw diagnostic replies"));
    assert!(!preview.contains("DEADBEEF"));
    assert!(!preview.contains("Some("));
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
    selected.series.insert(999);
    assert!(
        recording
            .selection_preview(&selected)
            .unwrap_err()
            .contains("measurement")
    );
    selected.series.clear();
    selected.dtcs.insert(999);
    assert!(
        recording
            .selection_preview(&selected)
            .unwrap_err()
            .contains("DTC")
    );
}

#[test]
fn empty_recordings_missing_files_wrong_types_and_versions() {
    let f = Fixture::new();
    let empty = Recording::open(f.path.clone()).unwrap();
    assert!(empty.overview.series.is_empty());
    assert!(empty.dtcs.is_empty());
    assert_eq!(Selection::all(&empty), Selection::default());
    assert!(
        Recording::open(f.path.with_extension("sqlite"))
            .unwrap_err()
            .contains("extension")
    );
    let missing = f
        .path
        .with_file_name("kittx-analyzer-nonexistent-fixture.kittx");
    assert!(!missing.exists());
    assert!(Recording::open(missing.clone()).is_err());
    assert!(!missing.exists());
    f.db.pragma_update(None, "user_version", 2).unwrap();
    assert!(Recording::open(f.path.clone()).is_err());
    f.db.pragma_update(None, "user_version", 1).unwrap();
    f.db.pragma_update(None, "application_id", 0).unwrap();
    assert!(Recording::open(f.path.clone()).is_err());
}

#[test]
fn labels_cover_decoded_measurements_and_preserve_unknown_identifiers() {
    for (id, name) in [
        ("01:04", "Engine load"),
        ("01:05", "Coolant temperature"),
        ("01:0B", "Manifold pressure"),
        ("01:0C", "Engine RPM"),
        ("01:0D", "Vehicle speed"),
        ("01:0F", "Intake temperature"),
        ("01:11", "Throttle position"),
        ("adapter:voltage", "Adapter battery voltage"),
        ("vendor:torque", "vendor:torque"),
    ] {
        assert_eq!(signal_label(id), name);
    }
}
