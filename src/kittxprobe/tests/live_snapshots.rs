#[path = "../../../tests/common/viewer_fixture.rs"]
mod fixture;
use fixture::Fixture;
use kittx_probe::live::LiveSnapshot;
use kittx_probe::live::bounded_view;
use kittx_viewer::model::Viewport;
use rusqlite::params;

#[test]
fn navigation_is_bounded_without_shrinking_the_window_at_recording_edges() {
    let full = Viewport::new(100, 1100);
    assert_eq!(
        bounded_view(Viewport::new(0, 250), full),
        Viewport::new(100, 350)
    );
    assert_eq!(
        bounded_view(Viewport::new(1000, 1250), full),
        Viewport::new(850, 1100)
    );
    assert_eq!(bounded_view(Viewport::new(-1000, 2000), full), full);
    assert_eq!(
        bounded_view(Viewport::new(300, 600), full),
        Viewport::new(300, 600)
    );
    assert_eq!(bounded_view(Viewport::new(i64::MIN, i64::MAX), full), full);
    let end = Viewport::new(i64::MAX - 1000, i64::MAX);
    assert_eq!(
        bounded_view(Viewport::new(i64::MAX - 100, i64::MAX), end),
        Viewport::new(i64::MAX - 100, i64::MAX)
    );
}

#[test]
fn custom_view_reloads_original_samples_outside_rolling_window_and_keeps_dtc_history() {
    let f = Fixture::new();
    for second in 0..=120 {
        f.sample(second * 1000 + 1, second as f64);
    }
    for id in 20_002..20_100 {
        f.sample(id, 20.);
    }
    f.db.execute("UPDATE session SET committed_sequence=120001", [])
        .unwrap();
    f.scan(1, "ok", Some(1));
    let before = std::fs::read(&f.path).unwrap();
    let automatic = LiveSnapshot::read(&f.path, true, 1).unwrap();
    let full = LiveSnapshot::read(&f.path, false, 1).unwrap();
    assert_eq!(automatic.full, full.view);
    let requested = Viewport::new(20_010_000, 20_030_000);
    assert!(requested.to < automatic.view.from);
    let detail = LiveSnapshot::read_view(&f.path, true, 100, Some(requested)).unwrap();
    assert_eq!(detail.view, requested);
    assert_eq!(detail.full, full.view);
    let points: Vec<_> = detail.tracks[0].signals[0]
        .points
        .iter()
        .filter(|p| p.elapsed_us >= requested.from && p.elapsed_us <= requested.to)
        .collect();
    assert_eq!(points.len(), 21);
    assert!(points.iter().any(|p| p.elapsed_us == 20_020_000));
    assert!(
        !full.tracks[0].signals[0]
            .points
            .iter()
            .any(|p| p.elapsed_us == 20_020_000)
    );
    assert_eq!(detail.dtcs[0].codes, ["P0123"]);
    let zoom_out =
        LiveSnapshot::read_view(&f.path, true, 100, Some(Viewport::new(-100, 200_000_000)))
            .unwrap();
    assert_eq!(zoom_out.view, full.view);
    let reset = LiveSnapshot::read_view(&f.path, true, 100, None).unwrap();
    assert_eq!(reset.view, automatic.view);
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
}

fn raw(f: &Fixture, id: i64, pid: u8, data: &[u8], ecu: i64) {
    f.sample(id, 0.);
    let signal = format!("01:{pid:02X}:raw");
    f.db.execute(
        "INSERT OR IGNORE INTO signal_definition VALUES(?1,1,?2,'raw','1',1)",
        params![signal, pid],
    )
    .unwrap();
    f.db.execute("UPDATE measurement SET signal_id=?1,unit='raw',value_type='blob',number_value=NULL,blob_value=?2,ecu_id=?3 WHERE id=?4",params![signal,[vec![0x41,pid],data.to_vec()].concat(),ecu,id]).unwrap();
}

#[test]
fn all_560_numeric_fields_are_grouped_by_unit_and_ecu_identity_is_preserved() {
    let f = Fixture::new();
    let rows: Vec<Vec<_>> = include_str!("../../viewerui/pids/fields.tsv")
        .lines()
        .filter(|s| !s.starts_with('#'))
        .map(|s| s.split('\t').collect())
        .collect();
    let mut blocks = std::collections::BTreeMap::<u8, Vec<u8>>::new();
    for r in &rows {
        let end = r[4].parse::<usize>().unwrap() + r[5].parse::<usize>().unwrap();
        let bytes = blocks
            .entry(u8::from_str_radix(r[0], 16).unwrap())
            .or_default();
        bytes.resize(bytes.len().max(end.div_ceil(8)), 0x55);
    }
    for r in rows {
        for (column, on) in [(12, true), (13, false)] {
            for bit in r[column].split(',').filter(|s| !s.is_empty()) {
                let bit = bit.parse::<usize>().unwrap();
                let b = &mut blocks
                    .get_mut(&u8::from_str_radix(r[0], 16).unwrap())
                    .unwrap()[bit / 8];
                if on {
                    *b |= 1 << (7 - bit % 8);
                } else {
                    *b &= !(1 << (7 - bit % 8));
                }
            }
        }
    }
    for (id, (pid, data)) in blocks.iter().enumerate() {
        raw(&f, id as i64 + 1, *pid, data, 1);
    }
    let before = std::fs::read(&f.path).unwrap();
    let snapshot = LiveSnapshot::read(&f.path, true, 500).unwrap();
    assert_eq!(snapshot.tracks.len(), 27);
    assert_eq!(
        snapshot
            .tracks
            .iter()
            .map(|t| t.signals.len())
            .sum::<usize>(),
        560
    );
    assert_eq!(snapshot.raw_series, 195);
    assert!(snapshot.dtcs.is_empty());
    for track in &snapshot.tracks {
        for signal in &track.signals {
            assert_eq!(
                kittx_viewer::pid::field(&signal.signal).unwrap().unit,
                track.unit
            );
            assert_eq!(signal.sender, "7E8");
            assert_eq!(signal.segment, 1);
            assert_eq!(signal.points.len(), 1);
        }
    }
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
    raw(&f, 196, 0x42, &[0x30, 0x70], 2);
    let newer = LiveSnapshot::read(&f.path, false, 500).unwrap();
    let voltage = newer.tracks.iter().find(|t| t.unit == "V").unwrap();
    assert!(voltage.signals.iter().any(|s| s.sender == "7E8"));
    assert!(
        voltage
            .signals
            .iter()
            .any(|s| s.sender == "7E9" && s.points[0].value == 12.4)
    );
    assert_eq!(snapshot.cutoff, 195);
    assert_eq!(newer.cutoff, 196);
}

#[test]
fn rolling_window_final_full_interval_raw_status_and_dtc_outcomes() {
    let f = Fixture::new();
    f.sample(1, 40.);
    raw(&f, 70_000, 0x67, &[3, 120, 110], 1);
    raw(&f, 70_001, 0x51, &[1], 1);
    f.scan(1, "ok", Some(1));
    f.scan(70_000, "ok", Some(0));
    f.scan(70_001, "failed", None);
    f.db.execute_batch("UPDATE dtc_scan_result SET category='pending' WHERE id=70000;UPDATE dtc_scan_result SET category='permanent',error='NO DATA',ecu_id=NULL WHERE id=70001;").unwrap();
    let before = std::fs::read(&f.path).unwrap();
    let live = LiveSnapshot::read(&f.path, true, 100).unwrap();
    let stopped = LiveSnapshot::read(&f.path, false, 100).unwrap();
    assert_eq!(live.view.to - live.view.from, 60_000_000);
    assert!(stopped.view.from < live.view.from);
    assert_eq!(live.tracks.len(), 2);
    assert_eq!(live.raw_series, 2);
    assert_eq!(live.dtcs.len(), 3);
    assert_eq!(live.dtcs[0].scan.category, "permanent");
    assert_eq!(live.dtcs[0].scan.outcome, "failed");
    assert_eq!(live.dtcs[0].sender, "unknown");
    assert!(live.dtcs[0].codes.is_empty());
    assert_eq!(live.dtcs[1].scan.code_count, Some(0));
    assert_eq!(live.dtcs[2].codes, ["P0123"]);
    assert!(!live.dtc_history_limited);
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
}

#[test]
fn dtc_only_history_is_bounded_to_latest_reads_without_losing_recorded_data() {
    let f = Fixture::new();
    for id in 1..=205 {
        f.sample(id, 0.);
        f.scan(id, "ok", Some(1));
    }
    f.db.execute("DELETE FROM measurement", []).unwrap();
    let snapshot = LiveSnapshot::read(&f.path, true, 100).unwrap();
    assert!(snapshot.tracks.is_empty());
    assert!(snapshot.dtc_history_limited);
    assert_eq!(snapshot.dtcs.len(), 200);
    assert_eq!(snapshot.dtcs.first().unwrap().scan.id, 205);
    assert_eq!(snapshot.dtcs.last().unwrap().scan.id, 6);
    assert_eq!(
        f.db.query_row("SELECT COUNT(*) FROM dtc_scan_result", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        205
    );
}

#[test]
fn long_session_plots_are_bounded_preserve_partial_ecus_and_release_wal_readers() {
    let f = Fixture::new();
    f.db.pragma_update(None, "journal_mode", "WAL").unwrap();
    for minute in 1..=600 {
        f.sample(minute * 60_000, minute as f64);
    }
    f.db.execute("UPDATE measurement SET ecu_id=2 WHERE id=600000", [])
        .unwrap();
    f.db.execute(
        "UPDATE acquisition SET outcome='partial' WHERE id=600000",
        [],
    )
    .unwrap();
    let before = std::fs::read(&f.path).unwrap();
    let live = LiveSnapshot::read(&f.path, true, 40).unwrap();
    let full = LiveSnapshot::read(&f.path, false, 40).unwrap();
    assert_eq!(full.view.to - full.view.from, 599 * 60_000_000);
    assert_eq!(live.view.to - live.view.from, 60_000_000);
    assert_eq!(full.tracks.len(), 1);
    assert_eq!(full.tracks[0].signals.len(), 2);
    assert!(
        full.tracks[0]
            .signals
            .iter()
            .map(|s| s.points.len())
            .sum::<usize>()
            <= 40 * 4 + 4
    );
    let peer = full.tracks[0]
        .signals
        .iter()
        .find(|s| s.sender == "7E9")
        .unwrap();
    assert_eq!(peer.points.len(), 1);
    assert!(peer.points[0].partial);
    assert_eq!(peer.points[0].elapsed_us, 600_000_000);
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
    let busy: i64 =
        f.db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
            .unwrap();
    assert_eq!(
        busy, 0,
        "live snapshots must release their WAL read transactions"
    );
}
