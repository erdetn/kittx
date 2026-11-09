mod common;
use common::*;
use kittx::{command::*, obd::*, response::Record, *};
use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
fn run(c: &mut Client, command: Command) -> Result<Response> {
    let mut result = None;
    c.execute(command, None, |r| result = Some(r));
    result.unwrap()
}

#[test]
fn convenience_profile_has_only_documented_safe_steps() {
    let profile = Initialization::obd(Protocol::Automatic);
    let wire: Vec<_> = profile
        .steps
        .iter()
        .map(|c| String::from_utf8(c.encode(13).unwrap()).unwrap())
        .collect();
    assert_eq!(
        wire,
        [
            "ATI\r", "ATE0\r", "ATL0\r", "ATS1\r", "ATH1\r", "ATD0\r", "ATCAF1\r", "ATR1\r",
            "ATCEA\r", "ATTPA0\r"
        ]
    );
    assert!(profile.steps.iter().all(|c| !matches!(
        c,
        Command::Vehicle { .. }
            | Command::Parameter { .. }
            | Command::StoreIdentifier(_)
            | Command::Fixed(Fixed::Reset)
    )));
}

#[test]
fn delayed_startup_does_not_shift_initialization_replies() {
    // Startup is withheld until after both synchronization probes. Each byte
    // arrives separately, as it can on USB serial or a fragmented TCP stream.
    for (version, firmware, baud) in [
        ("2.0", Firmware::V20, None),
        ("2.0", Firmware::V20, Some(38400)),
        ("2.1", Firmware::V21, None),
        ("2.1", Firmware::V21, Some(38400)),
    ] {
        for echo in [false, true] {
            let identity = format!("ELM327 v{version}\r>");
            let reply = if echo {
                format!("{identity} ATRV\r12.4V\r>")
            } else {
                format!("{identity}12.4V\r>")
            };
            let (mut client, trace) = client_with(
                vec![
                    step(" ", ""),
                    step("ATRV\r", &reply),
                    step("ATI\r", &identity),
                    step("ATE0\r", "OK\r>"),
                    step("ATRV\r", "12.6V\r>"),
                ],
                vec![],
                Initialization::new(vec![
                    Command::Fixed(Fixed::Identify),
                    Command::Toggle(Toggle::Echo, false),
                ]),
                baud,
            );
            let mut callbacks = 0;
            client.connect(None, |result| {
                callbacks += 1;
                result.unwrap();
            });
            assert_eq!(callbacks, 1);
            assert_eq!(client.firmware(), firmware);
            assert_eq!(client.settings().echo, Some(false));
            client.read_voltage(None, |result| {
                assert_eq!(result.unwrap().value, ResponseValue::Voltage(12.6));
            });
            assert_eq!(trace.lock().unwrap().writes, b" ATRV\rATI\rATE0\rATRV\r");
        }
    }
}

#[test]
fn echo_off_preserves_a_bus_reply_equal_to_the_request() {
    let (mut client, _) = client(vec![step("ATE0\r", "OK\r>"), step("A6\r", "A6\r>")]);
    connect(&mut client);
    run(&mut client, Command::Toggle(Toggle::Echo, false)).unwrap();
    client.send_vehicle(vec![0xA6], None, None, |result| {
        assert_eq!(
            result.unwrap().transaction.records,
            vec![Record::Hex(vec![0xA6])]
        );
    });
}
#[test]
fn init_applicability_success_and_no_body_failure() {
    let (mut c, trace) = client(vec![
        step("ATTP5\r", "OK\r>"),
        step("ATFI\r", "BUS INIT: OK\r>"),
        step("ATTP3\r", "OK\r>"),
        step("ATSI\r", "BUS INIT: ...OK\r>"),
        step("0100\r", ">"),
    ]);
    connect(&mut c);
    run(
        &mut c,
        Command::SetProtocol {
            protocol: Protocol::KwpFast,
            automatic: false,
            persist: false,
        },
    )
    .unwrap();
    assert!(run(&mut c, Command::Fixed(Fixed::SlowInit)).is_err());
    run(&mut c, Command::Fixed(Fixed::FastInit)).unwrap();
    assert_eq!(c.settings().active_protocol, Some(true));
    run(
        &mut c,
        Command::SetProtocol {
            protocol: Protocol::Iso9141,
            automatic: false,
            persist: false,
        },
    )
    .unwrap();
    assert!(run(&mut c, Command::Fixed(Fixed::FastInit)).is_err());
    run(&mut c, Command::Fixed(Fixed::SlowInit)).unwrap();
    assert_eq!(
        run(
            &mut c,
            Command::Vehicle {
                bytes: vec![1, 0],
                responses: None
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::MalformedResponse
    );
    assert_eq!(
        trace.lock().unwrap().writes,
        b"ATTP5\rATFI\rATTP3\rATSI\r0100\r"
    );
}
#[test]
fn io_error_and_zero_write_do_not_retry_a_command() {
    let (mut c, trace) = client(vec![
        Step {
            expected: b"ATRV\r".to_vec(),
            reply: Reply::Failure(io::ErrorKind::ConnectionReset),
        },
        step(" ", "STOPPED\r>"),
    ]);
    connect(&mut c);
    c.read_voltage(None, |r| {
        let e = r.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Io(io::ErrorKind::ConnectionReset));
        assert_eq!(e.recovery, Recovery::Synchronized);
    });
    assert_eq!(trace.lock().unwrap().writes, b"ATRV\r ");
    let trace = Arc::new(Mutex::new(Trace::default()));
    let connector = FakeConnector {
        steps: vec![step("ATRV\r", "")],
        startup: b">".to_vec(),
        trace: trace.clone(),
        baud: None,
        max_write: 0,
        fail_open: false,
    };
    let mut c = Client::with_connector(
        TransportConfig::tcp("emulator", 35000),
        ClientOptions {
            recovery_timeout: Duration::from_millis(5),
            ..ClientOptions::default()
        },
        Initialization::default(),
        Box::new(connector),
    )
    .unwrap();
    connect(&mut c);
    c.read_voltage(None, |r| {
        let e = r.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Io(io::ErrorKind::WriteZero));
        assert_eq!(e.recovery, Recovery::ReconnectRequired);
    });
    assert!(trace.lock().unwrap().writes.is_empty());
}
#[test]
fn line_overflow_retains_partial_evidence_and_recovers_prompt() {
    let trace = Arc::new(Mutex::new(Trace::default()));
    let connector = FakeConnector {
        steps: vec![
            step("ATRV\r", "12345678901234\r>"),
            step("ATRV\r", "12.6V\r>"),
        ],
        startup: b">".to_vec(),
        trace,
        baud: None,
        max_write: 2,
        fail_open: false,
    };
    let mut c = Client::with_connector(
        TransportConfig::tcp("emulator", 35000),
        ClientOptions {
            max_line_bytes: 8,
            max_transaction_bytes: 64,
            ..ClientOptions::default()
        },
        Initialization::default(),
        Box::new(connector),
    )
    .unwrap();
    connect(&mut c);
    c.read_voltage(None, |r| {
        let e = r.unwrap_err();
        assert_eq!(e.kind, ErrorKind::BufferOverflow);
        assert_eq!(e.partial, b"123456789");
        assert_eq!(e.recovery, Recovery::Synchronized);
    });
    c.read_voltage(None, |r| {
        r.unwrap();
    });
}
#[test]
fn missing_advertised_support_sender_is_not_silently_ignored() {
    let profile = Initialization::obd(Protocol::Can11_500);
    let mut steps: Vec<_> = profile
        .steps
        .iter()
        .map(|command| Step {
            expected: command.encode(13).unwrap(),
            reply: Reply::Bytes(if *command == Command::Fixed(Fixed::Identify) {
                b"ELM327 v2.1\r>".to_vec()
            } else {
                b"OK\r>".to_vec()
            }),
        })
        .collect();
    steps.extend([
        step(
            "0100\r",
            "7E8 06 41 00 80 00 00 01 00\r7E9 06 41 00 80 00 00 01 00\r>",
        ),
        step("ATDPN\r", "6\r>"),
        step("0120\r", "7E8 06 41 20 80 00 00 00 00\r>"),
        step("ATDPN\r", "6\r>"),
    ]);
    let (mut c, _) = client_with(steps, b">".to_vec(), profile, None);
    connect(&mut c);
    c.discover_pids(None, |r| {
        let e = r.unwrap_err();
        assert_eq!(e.kind, ErrorKind::IncompleteMessage);
        assert!(String::from_utf8(e.partial).unwrap().contains("7E8"));
    });
}
#[test]
fn malformed_vin_frames_and_dlc_records() {
    let sender = Sender::Can(0x7E8);
    let mut bytes = vec![0x49, 2, 1];
    bytes.extend_from_slice(b"1D4GP00R55B123456");
    let valid = EcuMessage {
        sender,
        bytes: bytes.clone(),
    };
    assert!(decode_vins(std::slice::from_ref(&valid), true).is_ok());
    bytes[3] = b'I';
    assert!(decode_vins(&[EcuMessage { sender, bytes }], true).is_err());
    let mut bytes = valid.bytes;
    bytes[2] = 2;
    assert!(decode_vins(&[EcuMessage { sender, bytes }], true).is_err());
    assert_eq!(
        Record::parse("7E8 4 03 41 05 7B"),
        Record::CanFrame {
            id: 0x7E8,
            dlc: Some(4),
            bytes: vec![3, 0x41, 5, 0x7B]
        }
    );
    assert!(matches!(
        Record::parse("7E8 9 03 41 05 7B"),
        Record::Unknown(_)
    ));
    let ms = reassemble(
        &transaction(&["C0 F1 10 03 41 05 7B 85"]),
        Layout::Legacy(Protocol::KwpFast),
    )
    .unwrap();
    assert_eq!(ms[0].bytes, [0x41, 5, 0x7B]);
    assert!(
        reassemble(
            &transaction(&["7E8 8 03 41 05 7B"]),
            Layout::Can {
                extended_id: false,
                dlc: true,
                extended_address: None
            }
        )
        .is_err()
    );
}
#[cfg(all(feature = "serial", not(target_os = "android")))]
#[test]
fn serial_open_failure_is_reported_without_hardware() {
    let config = TransportConfig::Serial(SerialConfig::new(
        std::env::temp_dir().join("kittx-nonexistent-serial-device"),
        38400,
    ));
    let mut c = Client::new(config);
    c.connect(None, |r| {
        assert!(matches!(r.unwrap_err().kind, ErrorKind::Io(_)))
    });
    assert_eq!(c.state(), SessionState::Disconnected);
}
