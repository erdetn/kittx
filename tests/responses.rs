mod common;
use common::transaction;
use kittx::{
    ErrorKind,
    command::{Command, Fixed},
    response::*,
};

#[test]
fn every_record_category_and_fault() {
    assert_eq!(Record::parse("OK"), Record::Ack);
    assert!(matches!(Record::parse("ELM327 v2.1"), Record::Identity(_)));
    for s in ["SEARCHING...", "BUS INIT: ...OK", "BUS INIT: OK", "STOPPED"] {
        assert!(matches!(Record::parse(s), Record::Status(_)));
    }
    for s in ["ACT ALERT", "!ACT ALERT", "LP ALERT", "!LP ALERT"] {
        assert!(matches!(Record::parse(s), Record::Alert(_)));
    }
    for (s, kind) in [
        ("?", ErrorKind::Rejected),
        ("NO DATA", ErrorKind::NoData),
        ("BUFFER FULL", ErrorKind::BufferOverflow),
        ("LV RESET", ErrorKind::Reset),
        ("ERR94", ErrorKind::Reset),
    ] {
        let tx = transaction(&[s]);
        assert_eq!(tx.check().unwrap_err().kind, kind);
        assert!(!tx.check().unwrap_err().partial.is_empty());
    }
    for s in [
        "UNABLE TO CONNECT",
        "BUS BUSY",
        "BUS ERROR",
        "CAN ERROR",
        "DATA ERROR",
        "FB ERROR",
        "ERR01",
        "BUS INIT: ...ERROR",
        "BUS INIT: ERROR",
    ] {
        assert!(matches!(Record::parse(s), Record::Fault(_)));
        assert_eq!(
            transaction(&[s]).check().unwrap_err().kind,
            ErrorKind::BridgeFault
        );
    }
    for s in ["7E8 03 41 05 7B <DATA ERROR", "7E8 03 41 05 7B <RX ERROR"] {
        assert!(matches!(Record::parse(s), Record::Annotated { .. }));
        assert_eq!(
            transaction(&[s]).check().unwrap_err().kind,
            ErrorKind::MalformedResponse
        );
    }
    assert!(matches!(
        Record::parse("FC: 30 00 00"),
        Record::FlowControl(_)
    ));
    assert!(matches!(Record::parse("7E8 RTR"), Record::RemoteFrame(_)));
    assert_eq!(Record::parse("014"), Record::Length(20));
    assert_eq!(
        Record::parse("0: 49 02 01"),
        Record::Segment {
            index: 0,
            digits: 1,
            bytes: vec![0x49, 2, 1]
        }
    );
    assert!(matches!(
        Record::parse("01: 04 FF"),
        Record::Segment { digits: 2, .. }
    ));
    assert_eq!(
        Record::parse("410C1AF8"),
        Record::Hex(vec![0x41, 12, 0x1A, 0xF8])
    );
    assert_eq!(
        Record::parse("vendor ready"),
        Record::Unknown("vendor ready".into())
    );
}
#[test]
fn scalar_values_and_complete_response_structs() {
    let cases = [
        (Fixed::Voltage, "12.6V", ResponseValue::Voltage(12.6)),
        (Fixed::ReadData, "A5", ResponseValue::Byte(0xA5)),
        (Fixed::ActivityCount, "FF", ResponseValue::Byte(255)),
        (Fixed::Ignition, "ON", ResponseValue::Ignition(true)),
        (Fixed::Ignition, "OFF", ResponseValue::Ignition(false)),
        (
            Fixed::ProtocolNumber,
            "A6",
            ResponseValue::Protocol {
                protocol: kittx::Protocol::Can11_500,
                automatic: true,
            },
        ),
        (
            Fixed::ProtocolNumber,
            "A",
            ResponseValue::Protocol {
                protocol: kittx::Protocol::J1939,
                automatic: false,
            },
        ),
        (
            Fixed::Description,
            "OBDII to RS232 Interpreter",
            ResponseValue::Text("OBDII to RS232 Interpreter".into()),
        ),
        (
            Fixed::Identifier,
            "MYBOARD_9906",
            ResponseValue::Text("MYBOARD_9906".into()),
        ),
        (
            Fixed::Keywords,
            "94 94",
            ResponseValue::Text("94 94".into()),
        ),
    ];
    for (command, line, value) in cases {
        let tx = transaction(&[line]);
        let r = Response::parse(&Command::Fixed(command), tx.clone()).unwrap();
        assert_eq!(r.value, value);
        assert_eq!(r.transaction, tx);
    }
    for (s, v) in [
        ("ELM327 v2.0", Firmware::V20),
        ("ELM327 v2.1", Firmware::V21),
        ("ELM327 emulator", Firmware::Unknown),
    ] {
        assert_eq!(Firmware::parse(s), v);
        let r = Response::parse(&Command::Fixed(Fixed::Identify), transaction(&[s])).unwrap();
        assert_eq!(
            r.value,
            ResponseValue::Firmware {
                version: v,
                identity: s.into()
            }
        );
    }
    let r = Response::parse(
        &Command::Fixed(Fixed::BufferDump),
        transaction(&["05 C1 33 F1 3E 23 C4 00 00 10 F8 00 00"]),
    )
    .unwrap();
    assert_eq!(
        r.value,
        ResponseValue::Buffer {
            length: 5,
            bytes: [0xC1, 0x33, 0xF1, 0x3E, 0x23, 0xC4, 0, 0, 0x10, 0xF8, 0, 0]
        }
    );
    let r = Response::parse(
        &Command::Fixed(Fixed::ParameterSummary),
        transaction(&["00:FF F 01:00 N", "02:FFF"]),
    )
    .unwrap();
    assert_eq!(
        r.value,
        ResponseValue::Parameters(vec![
            ParameterValue {
                id: 0,
                value: 255,
                enabled: false
            },
            ParameterValue {
                id: 1,
                value: 0,
                enabled: true
            },
            ParameterValue {
                id: 2,
                value: 255,
                enabled: false
            }
        ])
    );
    assert_eq!(
        Response::parse(&Command::CanStatus, transaction(&["T:00 R:01"]))
            .unwrap()
            .value,
        ResponseValue::Text("T:00 R:01".into())
    );
    assert_eq!(
        Response::parse(&Command::RawAt("ATVENDOR".into()), transaction(&["CUSTOM"]))
            .unwrap()
            .value,
        ResponseValue::Raw
    );
    assert_eq!(
        Response::parse(&Command::Fixed(Fixed::Defaults), transaction(&["OK"]))
            .unwrap()
            .value,
        ResponseValue::Ack
    );
    assert_eq!(
        Response::parse(
            &Command::Fixed(Fixed::FastInit),
            transaction(&["BUS INIT: OK"])
        )
        .unwrap()
        .value,
        ResponseValue::Ack
    );
    assert_eq!(
        Response::parse(
            &Command::Vehicle {
                bytes: vec![1, 0],
                responses: None
            },
            transaction(&["41 00 FF FF FF FF"])
        )
        .unwrap()
        .value,
        ResponseValue::Bus
    );
}
#[test]
fn malformed_values_are_errors_not_panics() {
    for (command, line) in [
        (Fixed::Voltage, "nanV"),
        (Fixed::Voltage, "12.6"),
        (Fixed::Voltage, "-1V"),
        (Fixed::ReadData, "0102"),
        (Fixed::Ignition, "YES"),
        (Fixed::ProtocolNumber, "AD"),
        (Fixed::Identifier, "short"),
        (Fixed::BufferDump, "01 02"),
        (Fixed::ParameterSummary, "01:00 Q"),
        (Fixed::ParameterSummary, "01:éN"),
    ] {
        assert!(
            Response::parse(&Command::Fixed(command), transaction(&[line])).is_err(),
            "{command:?}"
        );
    }
    assert!(Response::parse(&Command::Fixed(Fixed::Defaults), transaction(&[])).is_err());
    assert!(
        Response::parse(
            &Command::Fixed(Fixed::Voltage),
            transaction(&["12.6V", "12.7V"])
        )
        .is_err()
    );
    assert_eq!(parse_hex("41 0c\t1A F8").unwrap(), [0x41, 12, 0x1A, 0xF8]);
    for s in ["", "0", "GG", "NO DATA", "é"] {
        assert!(parse_hex(s).is_err());
    }
}
