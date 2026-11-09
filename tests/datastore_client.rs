#![cfg(feature = "datastore")]
mod common;
use common::*;
use kittx::{
    command::{CanId, Header},
    datastore::*,
    obd::{DtcKind, Pid, Sender},
    *,
};
use std::time::Duration;
fn setup(mut rest: Vec<Step>) -> ClientSource {
    let mut steps = vec![
        step("ATI\r", "ELM327 v2.1\r>"),
        step("ATDPN\r", "6\r>"),
        step("ATH1\r", "OK\r>"),
        step("ATCAF1\r", "OK\r>"),
        step("ATD0\r", "OK\r>"),
        step("ATS1\r", "OK\r>"),
        step("ATR1\r", "OK\r>"),
        step("ATCEA\r", "OK\r>"),
    ];
    steps.append(&mut rest);
    let (client, _) = client(steps);
    let mut source = ClientSource::new(client);
    let info = source.connect(Duration::from_secs(1)).unwrap();
    assert_eq!(info.adapter, "ELM327 v2.1");
    assert!(info.transport.contains("emulator"));
    source
}
#[test]
fn evidence_method_and_source_decode_each_ecu_independently() {
    let mut source = setup(vec![
        step(
            "010C\r",
            "7E8 04 41 0C 1A F4 00 00 00\r7E9 03 7F 01 12 00 00 00 00\r>",
        ),
        step("ATDPN\r", "6\r>"),
        step("03\r", "7E8 04 43 01 13 37 00 00 00\r>"),
        step("ATDPN\r", "6\r>"),
        step("0100\r", "7E8 06 41 00 BE 3E B8 11 00\r>"),
        step("ATDPN\r", "6\r>"),
        step("0162\r", "7E8 03 41 62 80 00 00 00 00\r>"),
        step("ATDPN\r", "6\r>"),
        step("ATRV\r", "12.6V\r>"),
    ]);
    let result = source.read(
        &ReadRequest::Signal(Signal::Pid(Pid::EngineRpm)),
        Duration::from_secs(1),
    );
    assert_eq!(result.protocol, Some(Protocol::Can11_500));
    assert_eq!(result.responses[0].sender, Sender::Can(0x7E8));
    assert!(matches!(
        result.responses[0].value,
        ObservationValue::Number(1725.0)
    ));
    assert!(matches!(
        result.responses[1].value,
        ObservationValue::Failure { .. }
    ));
    assert!(result.raw_response.unwrap().starts_with(b"7E8 04"));
    let codes = source.read(&ReadRequest::Dtcs(DtcKind::Stored), Duration::from_secs(1));
    if let ObservationValue::Dtcs(codes) = &codes.responses[0].value {
        assert_eq!(codes[0].text, "P1337");
        assert_eq!(codes[0].bytes, [0x13, 0x37]);
    } else {
        panic!("DTC value");
    }
    let support = source.read(&ReadRequest::Supported(0), Duration::from_secs(1));
    assert!(matches!(
        support.responses[0].value,
        ObservationValue::Supported(0xBE3EB811)
    ));
    let raw = source.read(
        &ReadRequest::Signal(Signal::RawPid(0x62)),
        Duration::from_secs(1),
    );
    assert!(matches!(&raw.responses[0].value,ObservationValue::Raw(v) if v==&[0x41,0x62,0x80]));
    let voltage = source.read(
        &ReadRequest::Signal(Signal::AdapterVoltage),
        Duration::from_secs(1),
    );
    assert!(matches!(
        voltage.responses[0].value,
        ObservationValue::Number(12.6)
    ));
    assert_eq!(source.client().firmware(), Firmware::V21);
    source.disconnect(Duration::from_secs(1)).unwrap();
}
#[test]
fn raw_profile_preserves_wire_and_restores_functional_addressing() {
    let mut source = setup(vec![
        step("010D\r", "7E8 03 41 0D 2A 00 00 00 00\r>"),
        step("ATDPN\r", "6\r>"),
        step("ATSH7E0\r", "OK\r>"),
        step("ATCRA7E8\r", "OK\r>"),
        step("1902FF\r", "7E8 07 59 02 FF 12 34 56 08\r>"),
        step("ATCRA\r", "OK\r>"),
        step("ATSH7DF\r", "OK\r>"),
    ]);
    source.read(
        &ReadRequest::Signal(Signal::Pid(Pid::VehicleSpeed)),
        Duration::from_secs(1),
    );
    let profile = DiagnosticProfile {
        id: "documented-module".into(),
        version: "1".into(),
        namespace: "test:raw".into(),
        request: vec![0x19, 2, 0xFF],
        header: Some(Header::Can(CanId::Standard(0x7E0))),
        receive_filter: Some("7E8".into()),
        target: Some(Sender::Can(0x7E8)),
    };
    let result = source.read(&ReadRequest::Diagnostic(profile), Duration::from_secs(1));
    assert!(result.error.is_none());
    assert!(matches!(&result.responses[0].value,ObservationValue::Raw(v) if v.starts_with(b"7E8")));
    assert_eq!(result.responses[0].sender, Sender::Can(0x7E8));
}
#[test]
fn request_evidence_completion_once_and_original_bytes() {
    let (mut client, _) = client(vec![
        step("ATDPN\r", "6\r>"),
        step("ATH1\r", "OK\r>"),
        step("ATCAF1\r", "OK\r>"),
        step("ATD0\r", "OK\r>"),
        step("ATS1\r", "OK\r>"),
        step("ATR1\r", "OK\r>"),
        step("ATCEA\r", "OK\r>"),
        step("010D\r", "7E8 03 41 0D 2A 00 00 00 00\r>"),
        step("ATDPN\r", "6\r>"),
    ]);
    connect(&mut client);
    let mut called = 0;
    client.request_obd_evidence(vec![1, 0x0D], None, |r| {
        called += 1;
        let evidence = r.unwrap();
        assert_eq!(evidence.raw, b"7E8 03 41 0D 2A 00 00 00 00\r>");
        assert_eq!(evidence.messages[0].bytes, [0x41, 0x0D, 0x2A]);
        assert_eq!(evidence.protocol, Protocol::Can11_500);
    });
    assert_eq!(called, 1);
}
