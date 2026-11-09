mod common;
use common::transaction;
use kittx::{ErrorKind, Protocol, obd::*};
fn msg(bytes: &[u8]) -> EcuMessage {
    EcuMessage {
        sender: Sender::Can(0x7E8),
        bytes: bytes.to_vec(),
    }
}
#[test]
fn all_initial_pid_formulas_units_and_boundaries() {
    let cases = [
        (Pid::EngineLoad, Unit::Percent, 0.0, 100.0),
        (Pid::CoolantTemperature, Unit::Celsius, -40.0, 215.0),
        (Pid::ManifoldPressure, Unit::Kilopascals, 0.0, 255.0),
        (Pid::EngineRpm, Unit::Rpm, 0.0, 16383.75),
        (Pid::VehicleSpeed, Unit::KilometresPerHour, 0.0, 255.0),
        (Pid::IntakeTemperature, Unit::Celsius, -40.0, 215.0),
        (Pid::ThrottlePosition, Unit::Percent, 0.0, 100.0),
    ];
    for (pid, unit, min, max) in cases {
        for (b, value) in [(0, min), (255, max)] {
            let mut bytes = vec![0x41, pid as u8, b];
            if pid == Pid::EngineRpm {
                bytes.push(b);
            }
            let r = pid.decode(&msg(&bytes)).unwrap();
            assert_eq!(
                r,
                Measurement {
                    sender: Sender::Can(0x7E8),
                    pid,
                    value,
                    unit
                }
            );
        }
        assert!(pid.decode(&msg(&[0x41, pid as u8])).is_err());
        assert!(pid.decode(&msg(&[0x42, pid as u8, 0])).is_err());
    }
    assert_eq!(
        Pid::EngineRpm
            .decode(&msg(&[0x41, 12, 0x1A, 0xF8]))
            .unwrap()
            .value,
        1726.0
    );
}
#[test]
fn bitmap_discovery_and_dtcs_all_categories() {
    let b = SupportedPids::decode(&msg(&[0x41, 0, 0x80, 0, 0, 1]), 0).unwrap();
    assert!(b.supports(1));
    assert!(b.supports(32));
    assert!(!b.supports(0));
    assert!(!b.supports(33));
    assert!(b.has_next_block());
    assert!(SupportedPids::decode(&msg(&[0x41, 0, 0, 0, 0, 0]), 1).is_err());
    let b = SupportedPids {
        sender: Sender::Unknown,
        base: 0xE0,
        bitmap: 1,
    };
    assert!(!b.has_next_block());
    for (first, system, text) in [
        (0x01, DtcSystem::Powertrain, "P0133"),
        (0x41, DtcSystem::Chassis, "C0133"),
        (0x81, DtcSystem::Body, "B0133"),
        (0xC1, DtcSystem::Network, "U0133"),
        (0xD0, DtcSystem::Network, "U1033"),
    ] {
        let code = Dtc::from_bytes([first, 0x33]);
        assert_eq!(code.system, system);
        assert_eq!(code.to_string(), text);
    }
    for kind in [DtcKind::Stored, DtcKind::Pending, DtcKind::Permanent] {
        let response = kind.service() + 0x40;
        let can = decode_dtcs(&msg(&[response, 1, 1, 0x33]), kind, true).unwrap();
        assert_eq!(can.kind, kind);
        assert_eq!(can.codes[0].to_string(), "P0133");
        let legacy = decode_dtcs(&msg(&[response, 1, 0x33, 0, 0]), kind, false).unwrap();
        assert_eq!(legacy.codes, can.codes);
        assert!(decode_dtcs(&msg(&[response, 2, 1, 0x33]), kind, true).is_err());
        assert!(decode_dtcs(&msg(&[response, 0, 1, 0x33]), kind, true).is_err());
    }
    assert_eq!(
        positive(&msg(&[0x7F, 1, 0x12]), 1).unwrap_err().kind,
        ErrorKind::EcuNegative {
            service: 1,
            code: 0x12
        }
    );
    assert!(positive(&msg(&[0x7F, 1]), 1).is_err());
    assert!(positive(&msg(&[]), 255).is_err());
}
#[test]
fn can_multiframe_vin_and_interleaved_ecus() {
    let tx = transaction(&[
        "7E8 10 14 49 02 01 31 44 34",
        "7E9 03 41 05 7B 00 00 00 00",
        "7E8 21 47 50 30 30 52 35 35",
        "7E8 22 42 31 32 33 34 35 36",
    ]);
    let ms = reassemble(
        &tx,
        Layout::Can {
            extended_id: false,
            dlc: false,
            extended_address: None,
        },
    )
    .unwrap();
    assert_eq!(ms.len(), 2);
    assert_eq!(ms[0].sender, Sender::Can(0x7E9));
    assert_eq!(ms[1].sender, Sender::Can(0x7E8));
    let vin = decode_vins(&ms[1..], true).unwrap();
    assert_eq!(
        vin[0],
        Vin {
            sender: Sender::Can(0x7E8),
            value: "1D4GP00R55B123456".into()
        }
    );
    let tx = transaction(&["18DAF110 8 F1 03 41 05 7B 00 00 00"]);
    let ms = reassemble(
        &tx,
        Layout::Can {
            extended_id: true,
            dlc: true,
            extended_address: Some(0xF1),
        },
    )
    .unwrap();
    assert_eq!(ms[0].bytes, [0x41, 5, 0x7B]);
    assert_eq!(ms[0].sender, Sender::Can(0x18DAF110));
    let tx = transaction(&["7E80341057B00000000"]);
    assert_eq!(
        reassemble(
            &tx,
            Layout::Can {
                extended_id: false,
                dlc: false,
                extended_address: None
            }
        )
        .unwrap()[0]
            .bytes,
        [0x41, 5, 0x7B]
    );
}
#[test]
fn formatted_and_legacy_vin_and_header_checksum() {
    let tx = transaction(&[
        "014",
        "0: 49 02 01 31 44 34",
        "1: 47 50 30 30 52 35 35",
        "2: 42 31 32 33 34 35 36",
    ]);
    let ms = reassemble(&tx, Layout::Formatted).unwrap();
    assert_eq!(
        decode_vins(&ms, true).unwrap()[0].value,
        "1D4GP00R55B123456"
    );
    let legacy = [
        "49 02 01 00 00 00 31",
        "49 02 02 44 34 47 50",
        "49 02 03 30 30 52 35",
        "49 02 04 35 42 31 32",
        "49 02 05 33 34 35 36",
    ];
    let ms = reassemble(&transaction(&legacy), Layout::Formatted).unwrap();
    assert_eq!(
        decode_vins(&ms, false).unwrap()[0].value,
        "1D4GP00R55B123456"
    );
    let tx = transaction(&["48 6B 10 41 00 BE 3E B8 11 FA"]);
    let ms = reassemble(&tx, Layout::Legacy(Protocol::J1850Vpw)).unwrap();
    assert_eq!(ms[0].sender, Sender::Legacy(0x10));
    assert_eq!(ms[0].bytes, [0x41, 0, 0xBE, 0x3E, 0xB8, 0x11]);
    let tx = transaction(&["83 F1 10 41 05 7B 45"]);
    let ms = reassemble(&tx, Layout::Legacy(Protocol::KwpFast)).unwrap();
    assert_eq!(ms[0].bytes, [0x41, 5, 0x7B]);
}
#[test]
fn invalid_frames_missing_duplicates_annotations_and_ambiguity() {
    let layout = Layout::Can {
        extended_id: false,
        dlc: false,
        extended_address: None,
    };
    for lines in [
        vec!["7E8 21 00 00"],
        vec!["7E8 10 14 49 02 01 31 44 34"],
        vec!["7E8 10 14 49 02 01 31 44 34", "7E8 22 00 00 00 00 00 00 00"],
        vec!["7E8 09 41 05 7B"],
        vec!["800 03 41 05 7B"],
        vec!["7E8 03 41 05 7B <RX ERROR"],
    ] {
        assert!(
            reassemble(&transaction(&lines), layout).is_err(),
            "{lines:?}"
        );
    }
    assert_eq!(
        reassemble(
            &transaction(&["014", "0: 49 02 01 31 44 34", "014"]),
            Layout::Formatted
        )
        .unwrap_err()
        .kind,
        ErrorKind::AmbiguousSender
    );
    assert!(reassemble(&transaction(&["014", "01: 01 02"]), Layout::Formatted).is_err());
    assert!(decode_vins(&[msg(&[0x49, 2, 1, b'A'])], true).is_err());
    assert!(decode_vins(&[], true).is_err());
    assert!(
        reassemble(
            &transaction(&["83 F1 10 41 05 7B 00"]),
            Layout::Legacy(Protocol::KwpFast)
        )
        .is_err()
    );
}
#[test]
fn isotp_sequence_wrap_and_padding() {
    let payload: Vec<u8> = (0..125).collect();
    let mut lines = vec![format!(
        "7E8 10 7D {}",
        payload[..6]
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    )];
    for (i, chunk) in payload[6..].chunks(7).enumerate() {
        let mut b = chunk.to_vec();
        b.resize(7, 0);
        lines.push(format!(
            "7E8 {:02X} {}",
            0x20 | ((i + 1) & 15),
            b.iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    let refs: Vec<_> = lines.iter().map(String::as_str).collect();
    let ms = reassemble(
        &transaction(&refs),
        Layout::Can {
            extended_id: false,
            dlc: false,
            extended_address: None,
        },
    )
    .unwrap();
    assert_eq!(ms[0].bytes, payload);
}
