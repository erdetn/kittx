use kittx::datastore::Signal;
use kittx_recorder::model::{CaptureSettings, capture_catalog};

#[test]
fn named_catalog_covers_documented_live_pids_without_changing_request_identity() {
    let catalog = capture_catalog();
    // Publicly verified blocks, including each member of the oxygen families.
    // Gaps are deliberately not advertised as named/standardized measurements.
    let expected: Vec<_> = (1..=0xdcu16)
        .filter(|pid| pid % 32 != 0 && ![0x02, 0xa9, 0xad, 0xca, 0xdb].contains(pid))
        .collect();
    for pid in expected {
        let item = catalog.iter().find(|item| item.id == pid).unwrap();
        assert!(!item.label.contains("unverified"), "{}", item.label);
        let expected_signal = if item.decoded {
            format!("01:{pid:02X}")
        } else {
            assert_eq!(item.signal, Signal::RawPid(pid as u8));
            assert!(item.label.contains(&format!("Mode 01 PID {pid:02X}")));
            format!("01:{pid:02X}:raw")
        };
        let config = CaptureSettings {
            selected: [pid].into(),
            dtcs: [false; 3],
            ..Default::default()
        }
        .config()
        .unwrap();
        assert_eq!(config.signals.len(), 1);
        assert_eq!(config.signals[0].signal.id(), expected_signal);
    }
    for (name, pid) in [
        ("Oxygen sensor 1 voltage and short-term", 0x14),
        ("Oxygen sensor 8 voltage and short-term", 0x1b),
        ("Oxygen sensor 1 equivalence ratio and voltage", 0x24),
        ("Oxygen sensor 8 equivalence ratio and voltage", 0x2b),
        ("Oxygen sensor 1 equivalence ratio and current", 0x34),
        ("Oxygen sensor 8 equivalence ratio and current", 0x3b),
        ("ECU control module voltage", 0x42),
        ("Engine oil temperature", 0x5c),
        ("SCR ammonia storage", 0x95),
        ("Odometer", 0xa6),
        ("High-voltage battery pack voltages", 0xb6),
        ("Electric motor torques", 0xcd),
        ("AECD 21-25", 0xdc),
    ] {
        let matches: Vec<_> = catalog
            .iter()
            .filter(|item| item.label.starts_with(name))
            .collect();
        assert_eq!(matches.len(), 1, "{name}");
        assert_eq!(matches[0].id, pid, "{name}");
    }
    let unknown = catalog.iter().find(|item| item.id == 0xfd).unwrap();
    assert!(unknown.label.contains("unassigned") || unknown.label.contains("Unassigned"));
    assert_eq!(unknown.signal, Signal::RawPid(0xfd));
    for pid in [0, 2, 0x20, 0x40, 0x60, 0x80, 0xa0, 0xc0, 0xe0] {
        assert!(catalog.iter().all(|item| item.id != pid));
        assert!(
            CaptureSettings {
                selected: [pid].into(),
                ..Default::default()
            }
            .config()
            .is_err()
        );
    }
}
