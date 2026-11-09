#[path = "../../../tests/common/viewer_fixture.rs"]
mod fixture;
use fixture::Fixture;
use kittx_viewer::{
    document::Document,
    model::{Viewport, gap},
    pid,
};
use rusqlite::params;

fn raw(f: &Fixture, id: i64, pid: u8, data: &[u8], ecu: i64) {
    f.sample(id, 0.);
    let signal = format!("01:{pid:02X}:raw");
    f.db.execute("DELETE FROM measurement WHERE id=?", [id])
        .unwrap();
    f.db.execute(
        "INSERT OR IGNORE INTO signal_definition VALUES(?1,1,?2,'raw','1',1)",
        params![signal, pid],
    )
    .unwrap();
    f.db.execute(
        "INSERT OR IGNORE INTO capture_selection VALUES(?1,?2,NULL,1000,0,NULL,'supported')",
        params![1000 + i64::from(pid), signal],
    )
    .unwrap();
    f.db.execute(
        "UPDATE acquisition SET request=?1 WHERE id=?2",
        params![format!("01{pid:02X}"), id],
    )
    .unwrap();
    f.db.execute(
        "INSERT INTO measurement VALUES(?1,?1,?2,?3,?4,?4,'blob',NULL,?5,'raw')",
        params![
            id,
            ecu,
            signal,
            id * 1000,
            [vec![0x41, pid], data.to_vec()].concat()
        ],
    )
    .unwrap();
}

fn index(d: &Document, pid: u8, key: &str, ecu: i64) -> usize {
    let signal = format!("01:{pid:02X}:decoded:{key}");
    d.overview
        .series
        .iter()
        .position(|s| s.signal == signal && s.ecu_id == ecu)
        .unwrap()
}

#[test]
fn all_numeric_pid_blocks_expand_to_named_fields_with_units_and_original_evidence() {
    let f = Fixture::new();
    let mut blocks = std::collections::BTreeMap::<u8, Vec<u8>>::new();
    // Construct complete blocks with all documented sensors enabled, no no-result
    // flags, and non-sentinel data. Engineering-value tests are in pid_decoding.rs.
    let rows: Vec<Vec<_>> = include_str!("../pids/fields.tsv")
        .lines()
        .filter(|s| !s.starts_with('#'))
        .map(|s| s.split('\t').collect())
        .collect();
    for r in &rows {
        let pid = u8::from_str_radix(r[0], 16).unwrap();
        let end = r[4].parse::<usize>().unwrap() + r[5].parse::<usize>().unwrap();
        let bytes = blocks.entry(pid).or_default();
        bytes.resize(bytes.len().max(end.div_ceil(8)), 0x55);
    }
    for r in &rows {
        let pid = u8::from_str_radix(r[0], 16).unwrap();
        for (column, on) in [(12, true), (13, false)] {
            for bit in r[column].split(',').filter(|s| !s.is_empty()) {
                let bit = bit.parse::<usize>().unwrap();
                let byte = &mut blocks.get_mut(&pid).unwrap()[bit / 8];
                if on {
                    *byte |= 1 << (7 - bit % 8);
                } else {
                    *byte &= !(1 << (7 - bit % 8));
                }
            }
        }
    }
    assert_eq!(blocks.len(), 195);
    for (id, (pid, data)) in blocks.iter().enumerate() {
        raw(&f, id as i64 + 1, *pid, data, 1);
    }
    let before = std::fs::read(&f.path).unwrap();
    let d = Document::open(f.path.clone()).unwrap();
    let selected: Vec<_> = d
        .overview
        .series
        .iter()
        .enumerate()
        .filter(|(_, s)| s.numeric)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(selected.len(), 560);
    assert_eq!(d.overview.series.len(), 560 + 195);
    for &i in &selected {
        let s = &d.overview.series[i];
        let field = pid::field(&s.signal).unwrap();
        assert_eq!(s.unit, field.unit);
        assert_eq!(s.count, 1);
        assert_eq!(s.sender, "7E8");
    }
    // More than the previous 256-series limit, with bounded envelope output.
    let frame = d
        .frame(d.full(), &selected, 100, Default::default())
        .unwrap();
    assert_eq!(frame.samples.len(), 560);
    assert!(frame.samples.iter().all(|(_, points)| points.len() == 1));
    let voltage = index(&d, 0x42, "VPWR", 1);
    assert!(
        d.inspect(d.full().from, &[voltage])
            .unwrap()
            .contains("Control module voltage")
    );
    assert!(d.raw_sample(voltage).unwrap().contains("41, 42"));
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
}

#[test]
fn invalid_unsupported_failed_and_missing_samples_break_lines_and_ecus_remain_separate() {
    let f = Fixture::new();
    raw(&f, 1, 0x67, &[3, 130, 120], 1);
    raw(&f, 2, 0x67, &[0, 130, 120], 1); // unsupported, not a pair of zero temperatures
    raw(&f, 3, 0x67, &[3, 131, 121], 1);
    raw(&f, 4, 0x67, &[3, 132], 1); // sensor 1 is complete; sensor 2 is missing
    raw(&f, 5, 0x67, &[3, 133, 123], 1);
    raw(&f, 6, 0x67, &[3, 110, 100], 2);
    raw(&f, 7, 0x67, &[3, 134, 124], 1);
    f.db.execute("UPDATE acquisition SET outcome='failed' WHERE id=6", [])
        .unwrap();
    raw(&f, 20, 0x67, &[3, 135, 125], 1); // exceeds 3 * scheduled period
    let d = Document::open(f.path.clone()).unwrap();
    let a = index(&d, 0x67, "ECT_1", 1);
    let b = index(&d, 0x67, "ECT_2", 1);
    let peer = index(&d, 0x67, "ECT_1", 2);
    assert_eq!(d.overview.series[a].count, 6);
    assert_eq!(d.overview.series[b].count, 5);
    assert_eq!(d.overview.series[peer].count, 1);
    let frame = d
        .frame(d.full(), &[a, b, peer], 800, Default::default())
        .unwrap();
    let pa = &frame.samples[0].1;
    let pb = &frame.samples[1].1;
    assert!(gap(&pa[0], &pa[1], None));
    assert!(!gap(&pa[1], &pa[2], None));
    assert!(gap(&pb[1], &pb[2], None));
    assert!(gap(&pa[3], &pa[4], None));
    assert!(gap(&pa[4], &pa[5], None));
    assert_eq!(frame.samples[2].1[0].value, 70.);
    assert!(frame.samples[2].1[0].partial);
    assert!(d.inspect(3500, &[a]).unwrap().contains("sample 3"));
    assert!(d.frame(d.full(), &[a, a], 100, Default::default()).is_err());
}

#[test]
fn raw_decoding_uses_the_frozen_cutoff_and_bounded_original_envelopes() {
    let f = Fixture::new();
    for i in 1..=300 {
        let millivolts: u16 = if i == 151 { 24000 } else { 12000 };
        raw(&f, i, 0x42, &millivolts.to_be_bytes(), 1);
    }
    raw(&f, 301, 0xfd, &[0x12, 0x34], 1);
    raw(&f, 302, 0x51, &[1], 1); // fuel-type code remains raw/categorical
    f.scan(151, "ok", Some(1));
    let d = Document::open(f.path.clone()).unwrap();
    let v = index(&d, 0x42, "VPWR", 1);
    raw(&f, 303, 0x42, &[0x75, 0x30], 1);
    let frame = d
        .frame(Viewport::new(0, 400000), &[v], 2, Default::default())
        .unwrap();
    assert!(frame.samples[0].1.len() <= 10);
    assert_eq!(frame.scans.len(), 1);
    assert!(
        frame.samples[0]
            .1
            .iter()
            .any(|p| p.id == 151 && p.value == 24.)
    );
    assert!(frame.samples[0].1.iter().all(|p| p.acquisition_id <= 302));
    assert!(d.inspect(303000, &[v]).unwrap().contains("sample 300"));
    let numeric: Vec<_> = d.overview.series.iter().filter(|s| s.numeric).collect();
    assert_eq!(numeric.len(), 1);
    assert_eq!(numeric[0].count, 300);
    assert!(
        d.overview
            .series
            .iter()
            .any(|s| s.signal == "01:FD:raw" && !s.numeric)
    );
    assert!(
        d.overview
            .series
            .iter()
            .any(|s| s.signal == "01:51:raw" && !s.numeric)
    );
    let inner = d
        .frame(Viewport::new(150500, 151500), &[v], 2, Default::default())
        .unwrap();
    assert_eq!(
        inner.samples[0].1.iter().map(|p| p.id).collect::<Vec<_>>(),
        [150, 151, 152]
    );
    f.db.execute("UPDATE session SET id='replacement'", [])
        .unwrap();
    assert!(d.inspect(0, &[v]).is_err());
    assert!(d.raw_sample(v).is_err());
}

#[test]
#[ignore = "requires KITTX_VIEW_RECORDING pointing to an existing recording; read-only"]
fn inspect_existing_pid_recording_without_modifying_it() {
    let path = std::path::PathBuf::from(std::env::var_os("KITTX_VIEW_RECORDING").unwrap());
    let before = std::fs::read(&path).unwrap();
    let started = std::time::Instant::now();
    let d = Document::open(path.clone()).unwrap();
    let selected: Vec<_> = d
        .overview
        .series
        .iter()
        .enumerate()
        .filter(|(_, s)| s.numeric)
        .map(|(i, _)| i)
        .collect();
    assert!(selected.len() > 100);
    println!(
        "Overview: {} numeric series in {:?}",
        selected.len(),
        started.elapsed()
    );
    let frame = d
        .frame(d.full(), &selected, 800, Default::default())
        .unwrap();
    assert!(frame.samples.iter().all(|(_, p)| !p.is_empty()));
    println!(
        "All-series frame: {} points, total {:?}",
        frame.samples.iter().map(|(_, p)| p.len()).sum::<usize>(),
        started.elapsed()
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
}
