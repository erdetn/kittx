use kittx::datastore::{DtcMarker, DtcScanRow};
use kittx_viewer::model::{MarkerKind, Viewport, hit_dtc_marker, plot_dtc_markers};

fn marker(
    id: i64,
    time: i64,
    ecu: Option<i64>,
    category: &str,
    outcome: &str,
    codes: Option<i64>,
) -> DtcMarker {
    DtcMarker {
        scan: DtcScanRow {
            id,
            group_id: id,
            acquisition_id: id,
            ecu_id: ecu,
            category: category.into(),
            namespace: "OBD".into(),
            profile_id: None,
            outcome: outcome.into(),
            code_count: codes,
            raw_payload: None,
            error: None,
            utc_us: 1_700_000_000_000_000 + time,
            elapsed_us: time,
        },
        scans: 1,
        last_us: time,
        bucket_start_us: time,
        bucket_end_us: time,
    }
}

#[test]
fn pixel_groups_preserve_categories_ecus_and_all_marker_kinds() {
    let mut stored = marker(1, 100, Some(1), "stored", "ok", Some(2));
    stored.scans = 3;
    stored.last_us = 105;
    let scans = vec![
        stored,
        marker(2, 101, Some(2), "pending", "ok", Some(1)),
        marker(3, 102, Some(1), "stored", "ok", Some(0)),
        marker(4, 103, None, "pending", "failed", None),
        marker(5, 104, Some(1), "manufacturer_raw", "raw_only", None),
    ];
    let points = plot_dtc_markers(&scans, Viewport::new(0, 1000), 1000.);
    assert_eq!(points.len(), 4);
    let positive = points
        .iter()
        .find(|p| p.kind == MarkerKind::Positive)
        .unwrap();
    assert_eq!(positive.id, 1);
    assert_eq!(positive.scans, 4);
    assert_eq!(positive.x, 100.);
    assert_eq!(positive.members.len(), 2);
    assert_eq!(positive.members[0].last_us, 105);
    assert_eq!(positive.members[1].scan.category, "pending");
    assert_eq!(positive.members[1].scan.ecu_id, Some(2));
    for kind in [MarkerKind::Empty, MarkerKind::Failed, MarkerKind::Raw] {
        assert_eq!(points.iter().filter(|p| p.kind == kind).count(), 1);
    }
}

#[test]
fn targets_follow_viewport_and_edges_and_reject_invalid_sizes() {
    let scans = vec![
        marker(1, 99, None, "stored", "ok", Some(1)),
        marker(2, 100, None, "stored", "ok", Some(1)),
        marker(3, 200, None, "stored", "ok", Some(1)),
        marker(4, 201, None, "stored", "ok", Some(1)),
    ];
    let view = Viewport::new(100, 200);
    let points = plot_dtc_markers(&scans, view, 100.);
    assert_eq!(
        points.iter().map(|p| (p.id, p.x)).collect::<Vec<_>>(),
        [(2, 6.), (3, 94.)]
    );
    assert_eq!(hit_dtc_marker(&points, 0., 88., 100.), Some(0));
    assert_eq!(hit_dtc_marker(&points, 100., 88., 100.), Some(1));
    let tiny = plot_dtc_markers(&scans, view, 4.);
    assert!(tiny.iter().all(|p| p.x == 2.));
    for width in [0., -1., f32::NAN, f32::INFINITY] {
        assert!(plot_dtc_markers(&scans, view, width).is_empty());
    }
}

#[test]
fn hit_testing_uses_painted_points_and_lane_not_a_time_tolerance() {
    let scans = vec![
        marker(1, 100, Some(1), "stored", "ok", Some(1)),
        marker(2, 500, Some(1), "stored", "ok", Some(1)),
        marker(3, 500, Some(1), "permanent", "failed", None),
    ];
    let points = plot_dtc_markers(&scans, Viewport::new(0, 1000), 1000.);
    assert_eq!(hit_dtc_marker(&points, 107., 188., 200.), Some(0));
    assert_eq!(hit_dtc_marker(&points, 110., 188., 200.), None);
    assert_eq!(hit_dtc_marker(&points, 100., 170., 200.), None);
    assert_eq!(hit_dtc_marker(&points, 300., 188., 200.), None);
    assert_eq!(hit_dtc_marker(&points, 500., 188., 200.), Some(2));
    assert_eq!(hit_dtc_marker(&points, f32::NAN, 188., 200.), None);
    assert_eq!(hit_dtc_marker(&[], 100., 188., 200.), None);
}
