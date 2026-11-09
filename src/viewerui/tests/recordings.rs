#[path = "../../../tests/common/viewer_fixture.rs"]
mod fixture;
use fixture::Fixture;
use kittx_viewer::{document::Document, model::*};
#[test]
fn document_open_plot_inspect_and_cluster() {
    let f = Fixture::new();
    for i in 1..=10 {
        f.sample(i, i as f64)
    }
    f.scan(2, "ok", Some(1));
    f.scan(3, "ok", Some(0));
    let before = std::fs::read(&f.path).unwrap();
    let doc = Document::open(f.path.clone()).unwrap();
    let frame = doc
        .frame(doc.full(), &[0], 800, Default::default())
        .unwrap();
    assert!(!frame.samples[0].1.is_empty());
    assert_eq!(frame.view, doc.full());
    assert_eq!(frame.scans.len(), 2);
    assert!(doc.inspect(2000, &[0]).unwrap().contains("sample 2"));
    let positive = frame
        .scans
        .iter()
        .find(|m| m.scan.code_count == Some(1))
        .unwrap();
    assert!(doc.scan_details(&positive.scan).unwrap().contains("P0123"));
    assert!(doc.cluster_details(positive).unwrap().contains("P0123"));
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
    assert!(
        doc.frame(doc.full(), &vec![0; 257], 800, Default::default())
            .is_err()
    );
}
#[test]
fn missing_wrong_version_and_empty_recordings() {
    let f = Fixture::new();
    let doc = Document::open(f.path.clone()).unwrap();
    assert!(doc.overview.series.is_empty());
    assert_eq!(doc.full().to - doc.full().from, 1_000_000);
    assert!(
        doc.frame(doc.full(), &[], 100, Default::default())
            .unwrap()
            .samples
            .is_empty()
    );
    f.db.pragma_update(None, "user_version", 2).unwrap();
    assert!(Document::open(f.path.clone()).is_err());
    assert!(Document::open(f.path.with_extension("missing")).is_err());
}
#[test]
fn defaults_labels_and_gap_identity() {
    assert_eq!(signal_label("01:0C"), "Engine RPM");
    assert_eq!(signal_label("vendor:123"), "vendor:123");
    let f = Fixture::new();
    for i in [1, 2, 9] {
        f.sample(i, i as f64)
    }
    let doc = Document::open(f.path.clone()).unwrap();
    let points = doc
        .frame(doc.full(), &[0], 100, Default::default())
        .unwrap()
        .samples
        .remove(0)
        .1;
    assert!(!gap(&points[0], &points[1], Some(1)));
    assert!(gap(&points[1], &points[2], Some(1)));
}

#[test]
fn raw_series_bytes_and_frozen_document() {
    let f = Fixture::new();
    f.sample(1, 1.);
    f.db.execute_batch(
        "INSERT INTO measurement VALUES(100,1,1,'raw',1000,1000,'blob',NULL,X'00FF','bytes');",
    )
    .unwrap();
    let doc = Document::open(f.path.clone()).unwrap();
    let index = doc
        .overview
        .series
        .iter()
        .position(|s| s.signal == "raw")
        .unwrap();
    assert!(doc.raw_sample(index).unwrap().contains("00, FF"));
    assert!(doc.raw_sample(100).is_err());
    f.sample(2, 2.);
    let index = doc.overview.series.iter().position(|s| s.numeric).unwrap();
    let frame = doc
        .frame(Viewport::new(0, 3000), &[index], 100, Default::default())
        .unwrap();
    assert_eq!(frame.samples[0].1.len(), 1);
}

#[test]
fn cancellation_and_invalid_series_are_results() {
    let f = Fixture::new();
    f.sample(1, 1.);
    let doc = Document::open(f.path.clone()).unwrap();
    assert!(
        doc.frame_with_cancel(doc.full(), &[0], 10, Default::default(), || true)
            .is_err()
    );
    assert!(
        doc.frame(doc.full(), &[999], 10, Default::default())
            .is_err()
    );
    assert!(doc.inspect(0, &[999]).is_err());
    f.db.execute("UPDATE session SET id='replacement'", [])
        .unwrap();
    assert!(doc.frame(doc.full(), &[0], 10, Default::default()).is_err());
}

#[test]
fn marker_semantics_are_distinct() {
    let f = Fixture::new();
    for i in 1..=4 {
        f.sample(i, 1.)
    }
    f.scan(1, "ok", Some(1));
    f.scan(2, "ok", Some(0));
    f.scan(3, "failed", None);
    f.scan(4, "raw_only", None);
    let doc = Document::open(f.path.clone()).unwrap();
    let frame = doc.frame(doc.full(), &[], 100, Default::default()).unwrap();
    let kinds: Vec<_> = frame.scans.iter().map(|m| marker_kind(&m.scan)).collect();
    assert_eq!(
        kinds,
        vec![
            MarkerKind::Positive,
            MarkerKind::Empty,
            MarkerKind::Failed,
            MarkerKind::Raw
        ]
    );
}

#[test]
fn selected_dtc_groups_show_codes_times_and_original_raw_payload() {
    let f = Fixture::new();
    f.db.execute("UPDATE session SET started_utc_us=created_utc_us+1000", [])
        .unwrap();
    for id in 1..=5 {
        f.sample(id, id as f64);
    }
    f.scan(2, "ok", Some(1));
    f.scan(3, "ok", Some(1));
    f.scan(4, "raw_only", None);
    f.db.execute_batch("UPDATE dtc_scan_result SET category='pending',ecu_id=2 WHERE id=3;UPDATE dtc_entry SET code='P0456',code_bytes=X'0456' WHERE id=3;UPDATE dtc_scan_result SET namespace='vendor',raw_payload=X'5902FF123456' WHERE id=4;").unwrap();
    let before = std::fs::read(&f.path).unwrap();
    let doc = Document::open(f.path.clone()).unwrap();
    let frame = doc
        .frame(doc.full(), &[], 1000, Default::default())
        .unwrap();
    let selected = doc.marker_details(&frame.scans[..2]).unwrap();
    assert!(selected.contains("Codes: P0123"));
    assert!(selected.contains("Codes: P0456"));
    assert!(selected.contains("Observed: 0.001000 s from recording start"));
    assert!(selected.contains("Observed: 0.002000 s from recording start"));
    assert!(selected.contains("UTC: 2000 µs"));
    assert!(selected.contains("ECU: 2 · result: ok"));
    assert!(!selected.contains("Some("));
    let raw = frame
        .scans
        .iter()
        .find(|marker| marker.scan.id == 4)
        .unwrap();
    assert!(raw.scan.raw_payload.is_none());
    let text = doc.marker_details(std::slice::from_ref(raw)).unwrap();
    assert!(text.contains("Codes: -"));
    assert!(text.contains("code count: -"));
    assert!(text.contains("59, 02, FF, 12, 34, 56"));
    assert_eq!(doc.marker_details(&[]).unwrap(), "");
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
    f.sample(6, 6.);
    f.scan(6, "ok", Some(1));
    let mut future = raw.clone();
    future.scan.id = 6;
    assert!(doc.marker_details(&[future]).is_err());
    let appended = std::fs::read(&f.path).unwrap();
    doc.marker_details(std::slice::from_ref(raw)).unwrap();
    assert_eq!(std::fs::read(&f.path).unwrap(), appended);
}

#[test]
fn dtc_cluster_selection_is_bounded_and_does_not_mix_unknown_ecu_with_known_ecus() {
    let f = Fixture::new();
    for id in 1..=105 {
        f.sample(id, id as f64);
        f.scan(id, "ok", Some(1));
    }
    f.db.execute_batch("UPDATE dtc_scan_result SET ecu_id=NULL WHERE id IN(1,2);UPDATE dtc_entry SET code='P9999' WHERE id>2;UPDATE acquisition SET operation_started_elapsed_us=1500,operation_completed_elapsed_us=1500 WHERE id=3;").unwrap();
    let doc = Document::open(f.path.clone()).unwrap();
    let frame = doc.frame(doc.full(), &[], 1, Default::default()).unwrap();
    let unknown = frame
        .scans
        .iter()
        .find(|marker| marker.scan.ecu_id.is_none())
        .unwrap();
    let text = doc.cluster_details(unknown).unwrap();
    assert_eq!(text.matches("DTC read\n").count(), 2);
    assert!(text.contains("ECU: -"));
    assert!(!text.contains("P9999"));
    let text = doc.marker_details(&frame.scans).unwrap();
    assert_eq!(text.matches("DTC read\n").count(), 100);
    assert!(text.contains("zoom in"));
}
