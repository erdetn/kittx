#![cfg(feature = "datastore")]
use kittx::datastore::*;
#[path = "common/viewer_fixture.rs"]
mod fixture;
use fixture::Fixture;
#[test]
fn overview_envelope_cutoff_nearest_and_validation() {
    let f = Fixture::new();
    f.db.execute_batch("BEGIN").unwrap();
    for i in 1..=2000 {
        f.sample(i, if i == 711 { 9999. } else { (i % 50) as f64 });
    }
    f.db.execute_batch("COMMIT").unwrap();
    let r = RecordingReader::open(&f.path).unwrap();
    let o = r.overview().unwrap();
    assert_eq!(o.session.vin, None);
    assert_eq!(o.origin_us, 1_000_000);
    assert_eq!(o.series.len(), 1);
    assert_eq!(o.series[0].sender, "7E8");
    let s = &o.series[0];
    let points = r.plot_samples(s, 2000, 0, 2_000_000, 10).unwrap();
    assert!(points.len() <= 42);
    assert!(points.iter().any(|p| p.value == 9999.));
    assert!(
        points
            .windows(2)
            .all(|p| p[0].elapsed_us <= p[1].elapsed_us)
    );
    assert!(points.iter().all(|p| p.run_id == 1));
    assert_eq!(r.nearest_sample(s, 2000, 711_001).unwrap().unwrap().id, 711);
    assert!(r.plot_samples(s, 2000, 10, 0, 1).is_err());
    assert!(r.plot_samples(s, 2000, 0, 10, 0).is_err());
    f.sample(2001, 10000.);
    assert!(
        !r.plot_samples(s, 2000, 0, 3_000_000, 10)
            .unwrap()
            .iter()
            .any(|p| p.id == 2001)
    );
    assert_eq!(r.overview().unwrap().session.committed_sequence, 2001);
}
#[test]
fn gaps_period_changes_and_failure_survive_decimation() {
    let f = Fixture::new();
    for i in [1, 2, 10, 11, 12, 13] {
        f.sample(i, i as f64)
    }
    f.db.execute("UPDATE acquisition SET outcome='failed' WHERE id=12", [])
        .unwrap();
    let r = RecordingReader::open(&f.path).unwrap();
    let s = &r.overview().unwrap().series[0];
    let p = r.plot_samples(s, 13, 0, 20000, 100).unwrap();
    assert_eq!(p[0].run_id, p[1].run_id);
    assert_ne!(p[1].run_id, p[2].run_id);
    assert!(p[4].partial);
    assert_ne!(p[3].run_id, p[4].run_id);
}
#[test]
fn dtc_clustering_includes_empty_failed_raw_and_uses_completed_time() {
    let f = Fixture::new();
    for i in 1..=6 {
        f.sample(i, i as f64)
    }
    f.scan(1, "ok", Some(1));
    f.scan(2, "ok", Some(1));
    f.scan(3, "ok", Some(0));
    f.scan(4, "failed", None);
    f.scan(5, "raw_only", None);
    f.scan(6, "ok", Some(1));
    let r = RecordingReader::open(&f.path).unwrap();
    let o = r.overview().unwrap();
    assert_eq!(o.dtc_occurrences, 3);
    assert_eq!(o.unique_codes, 1);
    let markers = r.dtc_markers(5, 0, 10000, 1).unwrap();
    assert_eq!(markers.iter().map(|m| m.scans).sum::<i64>(), 5);
    let positive = markers
        .iter()
        .find(|m| m.scan.code_count == Some(1))
        .unwrap();
    assert_eq!(positive.scans, 2);
    assert_eq!(positive.scan.elapsed_us, 1000);
    assert_eq!(positive.last_us, 2000);
    assert!(markers.iter().all(|m| m.scan.raw_payload.is_none()));
    assert!(r.dtc_markers(5, 10, 0, 1).is_err());
}
#[test]
fn wal_reader_releases_snapshot_and_never_updates_session() {
    let f = Fixture::new();
    f.db.pragma_update(None, "journal_mode", "WAL").unwrap();
    f.sample(1, 1.);
    let r = RecordingReader::open(&f.path).unwrap();
    let o = r.overview().unwrap();
    f.sample(2, 2.);
    let result: (i64, i64, i64) =
        f.db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(result.0, 0);
    assert_eq!(o.session.committed_sequence, 1);
    assert_eq!(r.session().unwrap().status, "recording");
    f.db.pragma_update(None, "user_version", 99).unwrap();
    assert!(RecordingReader::open(&f.path).is_err());
}

#[test]
fn dtc_projections_use_history_outside_view_and_ignore_failed_scans() {
    let f = Fixture::new();
    for i in 1..=6 {
        f.sample(i, i as f64)
    }
    f.scan(1, "ok", Some(1));
    f.scan(2, "failed", None);
    f.scan(3, "ok", Some(1));
    f.scan(4, "ok", Some(0));
    f.scan(5, "ok", Some(1));
    f.scan(6, "raw_only", None);
    let r = RecordingReader::open(&f.path).unwrap();
    let first = r
        .dtc_markers_mode(6, 0, 10000, 100, DtcDisplayMode::FirstObserved)
        .unwrap();
    assert_eq!(
        first.iter().map(|m| m.scan.id).collect::<Vec<_>>(),
        vec![1, 2, 6]
    );
    let change = r
        .dtc_markers_mode(6, 2500, 10000, 100, DtcDisplayMode::SetChanges)
        .unwrap();
    assert_eq!(
        change.iter().map(|m| m.scan.id).collect::<Vec<_>>(),
        vec![4, 5, 6]
    );
}

#[test]
fn catalog_keeps_raw_units_and_senders_separate() {
    let f = Fixture::new();
    f.sample(1, 42.);
    f.sample(2, 26.);
    f.db.execute_batch("UPDATE measurement SET unit='mph' WHERE id=2;INSERT INTO measurement VALUES(3,1,2,'speed',1000,1000,'number',84,NULL,'km/h');INSERT INTO measurement VALUES(4,1,1,'raw',1000,1000,'blob',NULL,X'00FF','bytes');").unwrap();
    let r = RecordingReader::open(&f.path).unwrap();
    let o = r.overview().unwrap();
    assert_eq!(o.series.len(), 4);
    assert_eq!(o.series.iter().filter(|s| !s.numeric).count(), 1);
    for s in o.series.iter().filter(|s| s.numeric) {
        let p = r.plot_samples(s, 2, 0, 2000, 20).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(
            p[0].value,
            if s.ecu_id == 2 {
                84.
            } else if s.unit == "mph" {
                26.
            } else {
                42.
            }
        );
    }
}
#[test]
fn six_hour_recording_has_bounded_points_and_period_history() {
    let f = Fixture::new();
    f.db.execute_batch("BEGIN").unwrap();
    for i in 1..=20000 {
        f.sample(i, (i % 100) as f64)
    }
    f.db.execute_batch("UPDATE measurement SET observed_elapsed_us=observed_elapsed_us*1000;UPDATE acquisition SET operation_started_elapsed_us=operation_started_elapsed_us*1000,operation_completed_elapsed_us=operation_completed_elapsed_us*1000;UPDATE capture_selection SET period_us=1000000;INSERT INTO selection_change VALUES(1,1,0,1000000000,2000000);COMMIT;").unwrap();
    let r = RecordingReader::open(&f.path).unwrap();
    let o = r.overview().unwrap();
    assert_eq!(o.last_us, 20_000_000_000);
    assert!(!o.policy.is_empty());
    let p = r
        .plot_samples(&o.series[0], 20000, 0, o.last_us, 500)
        .unwrap();
    assert!(p.len() <= 2002);
    assert!(p.iter().all(|p| p.run_id == 1));
}
