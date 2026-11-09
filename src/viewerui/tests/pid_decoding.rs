use kittx_viewer::{model::signal_label, pid};
use std::collections::BTreeSet;

fn decode(pid: u8, key: &str, data: &[u8]) -> Option<f64> {
    let field = pid::field(&format!("01:{pid:02X}:decoded:{key}")).unwrap();
    assert_eq!(field.source_signal(), format!("01:{pid:02X}:raw"));
    assert_eq!(signal_label(&field.signal()), field.name);
    field.decode(&[vec![0x41, pid], data.to_vec()].concat())
}

#[test]
fn catalog_covers_every_numeric_pid_in_pids_md_and_keeps_status_codes_categorical() {
    let mut identities = BTreeSet::new();
    for field in pid::fields() {
        assert!(identities.insert(field.signal()), "{}", field.signal());
        assert!(!field.name.is_empty() && !field.unit.is_empty());
        assert_eq!(pid::field(&field.signal()).unwrap().key, field.key);
        for invalid in [
            vec![],
            vec![0x41],
            vec![0x7f, field.pid, 0x12],
            vec![0x41, field.pid ^ 1, 0xff],
        ] {
            assert!(field.decode(&invalid).is_none());
        }
    }
    assert_eq!(identities.len(), 560);
    let ids: BTreeSet<_> = include_str!("../../../PIDS.MD")
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|line| line.get(..2))
        .filter_map(|hex| u8::from_str_radix(hex, 16).ok())
        .collect();
    assert_eq!(ids.len(), 209);
    let categorical = [
        0x03, 0x12, 0x13, 0x1c, 0x1d, 0x1e, 0x41, 0x51, 0x5f, 0x7d, 0x7e, 0x92, 0xc8, 0xd8,
    ]
    .into();
    let decoded: BTreeSet<_> = pid::fields().iter().map(|f| f.pid).collect();
    assert_eq!(
        ids.difference(&decoded).copied().collect::<BTreeSet<_>>(),
        categorical
    );
    assert_eq!(decoded.len(), 195);
    for invalid in [
        "vendor:123",
        "01:42:raw",
        "01:042:decoded:VPWR",
        "01:FD:decoded:unknown",
    ] {
        assert!(pid::field(invalid).is_none());
    }
}

#[test]
fn documented_transcripts_cover_scaling_signed_values_endian_and_sensor_offsets() {
    // Independent engineering values, rather than expectations calculated from
    // the bundled table. Includes each encoding/scaling family and late offsets.
    let cases: &[(u8, &str, &[u8], f64)] = &[
        (0x06, "SHRTFT1", &[128, 64], 0.),
        (0x06, "SHRTFT3", &[128, 64], -50.),
        (0x07, "LONGFT3", &[128, 192], 50.),
        (0x0a, "FP", &[100], 300.),
        (0x0c, "RPM", &[0x1a, 0xf4], 1725.),
        (0x10, "MAF", &[0x01, 0xf4], 5.),
        (0x14, "O2S11", &[100, 128], 0.5),
        (0x14, "SHRTFT11", &[100, 128], 0.),
        (0x1f, "RUNTM", &[1, 44], 300.),
        (0x22, "FRP_REL", &[0x03, 0xe8], 79.),
        (0x23, "FRP", &[0x00, 0x64], 1000.),
        (0x24, "LAMBDA11_VOLT", &[0x80, 0, 0x20, 0], 1.),
        (0x24, "O2S11_VOLT", &[0x80, 0, 0x20, 0], 1.),
        (0x32, "EVAP_VP", &[0xff, 0x00], -0.064),
        (0x34, "O2S11_CURRENT", &[0x80, 0, 0x7f, 0], -1.),
        (0x3c, "CATEMP11", &[0x03, 0xe8], 60.),
        (0x42, "VPWR", &[0x30, 0x70], 12.4),
        (0x43, "LOAD_ABS", &[0x01, 0xfe], 200.),
        (0x4f, "MAX_MAP", &[2, 8, 128, 100], 1000.),
        (0x50, "MAX_MAF", &[100], 1000.),
        (0x54, "EVAP_VP_WIDE", &[0xff, 0x9c], -0.1),
        (0x5c, "EOT", &[130], 90.),
        (0x5d, "FUEL_TIMING", &[0x69, 0x00], 0.),
        (0x5e, "FUEL_RATE", &[0, 100], 5.),
        (0x61, "TQ_DD", &[150], 25.),
        (0x63, "TQ_REF", &[1, 44], 300.),
        (0x66, "MAFB", &[3, 0, 32, 0, 64], 2.),
        (0x67, "ECT_2", &[3, 130, 120], 80.),
        (0x69, "EGR_B_ERR", &[0x3f, 0, 0, 128, 0, 0, 64], -50.),
        (0x6f, "TCIP_A", &[1, 100, 0], 100.),
        (0x6f, "TCIP_A", &[4, 100, 0], 800.),
        (0x73, "EP_11", &[1, 0x27, 0x10], 100.),
        (0x74, "TCA_RPM", &[1, 0x03, 0xe8], 10000.),
        (0x78, "EGT14", &[8, 0, 0, 0, 0, 0, 0, 0x03, 0xe8], 60.),
        (0x87, "MAP_B", &[2, 0, 0, 0x0c, 0x80], 100.),
        (0x95, "SCR_NH3_TGT_A", &[1, 0x27, 0x10], 1.),
        (0x97, "NOX_EO", &[1, 0x13, 0x88], 0.5),
        (0x98, "EGT18", &[8, 0, 0, 0, 0, 0, 0, 0x03, 0xe8], 60.),
        (0x9a, "HEV_BATT_V", &[2, 0, 0x57, 0x80, 0xff, 0x38], 350.),
        (0x9a, "HEV_BATT_A", &[4, 0, 0x57, 0x80, 0xff, 0x38], -20.),
        (0xa2, "CYL_RATE", &[1, 0], 8.),
        (0xa6, "ODO", &[0, 0x12, 0xd6, 0x87], 123456.7),
        (0xb3, "HVESS_ACR", &[0, 1, 0x90], 20.),
        (0xb5, "HVESS_Cur", &[0, 0x7d, 0], 0.),
        (0xb6, "HVESS_Volt", &[0, 0x1b, 0x58], 350.),
        (0xb9, "HEV_BATT_MAX_CELL_V", &[0x0c, 0xe4, 0x10, 0x68], 4.2),
        (0xbb, "HVESS_Enrg_In", &[0, 0, 0, 0x10, 0], 4096.),
        (0xbd, "HVESS_Enrg_Tput", &[0, 0, 0, 3, 0xe8], 100.),
        (0xbf, "HVESS_MIN_SOC", &[0, 0x7d, 0], 50.),
        (0xc2, "HVESS_DEnrg_CAP", &[0, 0, 0xc3, 0x50], 50.),
        (0xc7, "REFLASH_DIST", &[0x12, 0x34], 4660.),
        (0xcc, "EMA_RPM", &[1, 0xff, 0x9c], -100.),
        (0xcd, "EMA_TQ", &[1, 0xff, 0x38], -200.),
        (0xd3, "ODO_ENG", &[0, 0x12, 0xd6, 0x87], 123456.7),
        (0xd4, "HVESS_ENER", &[0xff, 0xff, 0xfc, 0x18], -0.0036),
        (0xd5, "FC_VOLT", &[1, 0x1b, 0x58], 350.),
        (0xda, "HVESS_CURR", &[0xff, 0xff, 0xfc, 0x18], -3.6),
        (0xdc, "AECD21_TIME1", &[1, 0, 1, 0, 0], 65536.),
    ];
    for &(pid, key, data, expected) in cases {
        let actual = decode(pid, key, data).unwrap_or_else(|| panic!("missing {pid:02X} {key}"));
        assert!(
            (actual - expected).abs() < 1e-8,
            "{pid:02X} {key}: {actual} != {expected}"
        );
    }
}

#[test]
fn unsupported_unavailable_and_truncated_data_are_gaps_and_complete_fields_survive() {
    assert_eq!(decode(0x67, "ECT_1", &[1, 130]), Some(90.));
    assert_eq!(decode(0x67, "ECT_2", &[1, 130, 140]), None);
    assert_eq!(decode(0x67, "ECT_2", &[3, 130]), None);
    assert_eq!(decode(0x67, "ECT_1", &[0, 130, 140]), None);
    assert_eq!(decode(0x14, "SHRTFT11", &[100, 255]), None);
    assert_eq!(decode(0x83, "NOX11", &[1, 0, 100]), Some(100.));
    assert_eq!(decode(0x83, "NOX11", &[0x11, 0, 100]), None);
    assert_eq!(decode(0x83, "NOX11", &[1, 255, 255]), None);
    assert_eq!(decode(0x6f, "TCIP_A", &[0, 100, 100]), None);
    assert_eq!(decode(0x68, "IAT_11", &[0, 0]), None);
    assert_eq!(decode(0xb6, "HVESPA_Volt", &[0, 0, 0, 0x1b, 0x58]), None);
    assert_eq!(
        decode(0xb6, "HVESPA_Volt", &[1, 0, 0, 0x1b, 0x58]),
        Some(350.)
    );
}

#[test]
fn valid_standard_endpoints_are_not_discarded_as_vehicle_specific_missing_values() {
    assert_eq!(decode(0x0c, "RPM", &[255, 255]), Some(16383.75));
    assert_eq!(decode(0x0d, "VSS", &[255]), Some(255.));
    assert_eq!(decode(0x2f, "FLI", &[0]), Some(0.));
    assert_eq!(decode(0x68, "IAT_11", &[1, 0]), Some(-40.));
}
