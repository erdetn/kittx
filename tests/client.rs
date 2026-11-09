mod common;
use common::*;
use kittx::{
    command::*,
    obd::{DtcKind, Pid, Sender},
    response::{Record, ResponseValue},
    *,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

fn execute(c: &mut Client, command: Command) -> Result<Response> {
    let mut result = None;
    c.execute(command, None, |r| result = Some(r));
    result.expect("callback invoked")
}
#[test]
fn constructors_configuration_connection_and_callback_thread() {
    let (mut c, trace) = client(vec![
        step("ATRV\r", "ATRV\r12.6V\r>"),
        step("ATI\r", "ELM327 v2.1\r>"),
    ]);
    assert_eq!(trace.lock().unwrap().opens, 0);
    assert_eq!(c.state(), SessionState::Disconnected);
    assert_eq!(c.config(), &TransportConfig::tcp("emulator", 35000));
    assert_eq!(
        c.options(),
        &ClientOptions {
            recovery_timeout: Duration::from_millis(10),
            ..ClientOptions::default()
        }
    );
    assert_eq!(c.settings().headers, None);
    assert_eq!(c.firmware(), Firmware::Unknown);
    let mut called = 0;
    let thread = std::thread::current().id();
    c.read_voltage(None, |r| {
        called += 1;
        assert_eq!(r.unwrap_err().kind, ErrorKind::Disconnected);
        assert_eq!(thread, std::thread::current().id());
    });
    assert_eq!(called, 1);
    connect(&mut c);
    assert_eq!(c.state(), SessionState::Unsynchronized);
    assert!(trace.lock().unwrap().writes.is_empty());
    c.connect(None, |r| {
        assert_eq!(r.unwrap_err().kind, ErrorKind::AlreadyConnected)
    });
    c.read_voltage(None, |r| {
        assert_eq!(r.unwrap().value, ResponseValue::Voltage(12.6));
        assert_eq!(thread, std::thread::current().id());
    });
    assert_eq!(c.state(), SessionState::Ready);
    assert_eq!(c.firmware(), Firmware::V21);
    assert_eq!(c.identity(), Some("ELM327 v2.1"));
    c.identify(None, |r| {
        assert!(matches!(r.unwrap().value, ResponseValue::Firmware { .. }))
    });
    c.disconnect(None, Result::unwrap);
    c.disconnect(Some(Duration::ZERO), Result::unwrap);
    assert_eq!(c.state(), SessionState::Disconnected);
    assert_eq!(trace.lock().unwrap().drops, 1);
    let c = Client::new(TransportConfig::tcp("127.0.0.1", 35000));
    assert_eq!(c.state(), SessionState::Disconnected);
    let c = Client::with_initialization(
        TransportConfig::tcp("127.0.0.1", 35000),
        Initialization::obd(Protocol::Automatic),
    );
    assert_eq!(c.state(), SessionState::Disconnected);
    assert!(Client::with_options(TransportConfig::tcp("", 0), ClientOptions::default()).is_err());
    assert!(
        ClientOptions {
            max_line_bytes: 0,
            ..ClientOptions::default()
        }
        .validate()
        .is_err()
    );
}
#[test]
fn optional_initialization_success_and_failure_cleanup() {
    let (mut c, trace) = client_with(
        vec![
            step("ATI\r", "ELM327 v2.0\r>"),
            step("ATE0\r", "ATE0\rOK\r>"),
        ],
        b">".to_vec(),
        Initialization::new(vec![
            Command::Fixed(Fixed::Identify),
            Command::Toggle(Toggle::Echo, false),
        ]),
        None,
    );
    connect(&mut c);
    assert_eq!(c.firmware(), Firmware::V20);
    assert_eq!(c.settings().echo, Some(false));
    assert_eq!(trace.lock().unwrap().writes, b"ATI\rATE0\r");
    let (mut c, trace) = client_with(
        vec![step("ATE0\r", "?\r>")],
        b">".to_vec(),
        Initialization::new(vec![Command::Toggle(Toggle::Echo, false)]),
        None,
    );
    c.connect(None, |r| {
        let e = r.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Initialization { step: 0 });
        assert_eq!(e.cause.unwrap().kind, ErrorKind::Rejected);
    });
    assert_eq!(c.state(), SessionState::Disconnected);
    assert_eq!(trace.lock().unwrap().drops, 1);
}
#[test]
fn empty_startup_safe_probe_and_explicit_resynchronization() {
    let (mut c, trace) = client_with(
        vec![
            step(" ", ""),
            step("ATRV\r", "ATRV\r12.0V\r>"),
            step("ATI\r", "ATI\rELM327 v2.1\r>"),
            step("ATRV\r", "12.0V\r>"),
            step(" ", "STOPPED\r>"),
            step("ATRV\r", "12.1V\r>"),
        ],
        vec![],
        Initialization::default(),
        None,
    );
    connect(&mut c);
    c.read_voltage(None, |r| {
        assert_eq!(r.unwrap().value, ResponseValue::Voltage(12.0))
    });
    c.resynchronize(None, Result::unwrap);
    c.read_voltage(None, |r| {
        assert_eq!(r.unwrap().value, ResponseValue::Voltage(12.1))
    });
    assert_eq!(trace.lock().unwrap().writes, b" ATRV\rATI\rATRV\r ATRV\r");
}
#[test]
fn zero_deadline_validation_and_firmware_gate_write_nothing() {
    let (mut c, trace) = client_with(vec![], vec![], Initialization::default(), None);
    c.connect(Some(Duration::ZERO), |r| {
        assert_eq!(r.unwrap_err().kind, ErrorKind::Timeout)
    });
    assert_eq!(trace.lock().unwrap().opens, 0);
    connect(&mut c);
    c.read_voltage(Some(Duration::ZERO), |r| {
        assert_eq!(r.unwrap_err().kind, ErrorKind::Timeout)
    });
    assert!(trace.lock().unwrap().writes.is_empty());
    c.assume_firmware(Firmware::V20);
    assert_eq!(
        execute(&mut c, Command::Timer(TimerMultiplier::Five))
            .unwrap_err()
            .kind,
        ErrorKind::UnsupportedFirmware
    );
    c.assume_firmware(Firmware::Unknown);
    assert_eq!(
        execute(&mut c, Command::Timer(TimerMultiplier::One))
            .unwrap_err()
            .kind,
        ErrorKind::UnknownCapability
    );
    for command in [
        Command::Byte(ByteSetting::BaudDivisor, 0),
        Command::Byte(ByteSetting::BaudDivisor, 1),
        Command::FlowMode(FlowMode::UserData),
        Command::Monitor(Monitor::All),
        Command::RawAt("ATLP".into()),
    ] {
        assert!(execute(&mut c, command).is_err());
    }
    assert!(trace.lock().unwrap().writes.is_empty());
}
#[test]
fn state_updates_require_ack_and_repeat_is_not_a_payload_byte() {
    let (mut c, trace) = client(vec![
        step("ATE0\r", "OK\r>"),
        step("ATH1\r", "?\r>"),
        step("ATH1\r", "OK\r>"),
        step("ATTP6\r", "OK\r>"),
        step("ATDPN\r", "6\r>"),
        step("010C1\r", "41 0C 1A F8\r>"),
        step("\r", "41 0C 1A F8\r>"),
    ]);
    connect(&mut c);
    execute(&mut c, Command::Toggle(Toggle::Echo, false)).unwrap();
    assert_eq!(c.settings().echo, Some(false));
    assert_eq!(
        execute(&mut c, Command::Toggle(Toggle::Headers, true))
            .unwrap_err()
            .kind,
        ErrorKind::Rejected
    );
    assert_eq!(c.settings().headers, None);
    execute(&mut c, Command::Toggle(Toggle::Headers, true)).unwrap();
    c.select_protocol(Protocol::Can11_500, false, false, None, |r| {
        r.unwrap();
    });
    execute(&mut c, Command::Fixed(Fixed::ProtocolNumber)).unwrap();
    assert_eq!(c.settings().protocol, Some(Protocol::Can11_500));
    c.send_vehicle(vec![1, 12], Some(1), None, |r| {
        assert_eq!(r.unwrap().value, ResponseValue::Bus)
    });
    execute(&mut c, Command::Repeat).unwrap();
    assert!(trace.lock().unwrap().writes.ends_with(b"010C1\r\r"));
}
#[test]
fn timeout_partial_data_recovery_and_reconnect_required() {
    let (mut c, trace) = client(vec![
        step("ATRV\r", "12."),
        step(" ", "STOPPED\r>"),
        step("ATRV\r", "12.8V\r>"),
    ]);
    connect(&mut c);
    c.read_voltage(None, |r| {
        let e = r.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Timeout);
        assert_eq!(e.partial, b"12.");
        assert_eq!(e.recovery, Recovery::Synchronized);
    });
    assert_eq!(c.state(), SessionState::Ready);
    c.read_voltage(None, |r| {
        assert_eq!(r.unwrap().value, ResponseValue::Voltage(12.8))
    });
    assert_eq!(trace.lock().unwrap().writes, b"ATRV\r ATRV\r");
    let (mut c, _) = client(vec![step("ATRV\r", ""), step(" ", ""), step("ATRV\r", "")]);
    connect(&mut c);
    c.read_voltage(None, |r| {
        assert_eq!(r.unwrap_err().recovery, Recovery::ReconnectRequired)
    });
    assert_eq!(c.state(), SessionState::Unusable);
    assert_eq!(
        execute(&mut c, Command::Fixed(Fixed::Identify))
            .unwrap_err()
            .kind,
        ErrorKind::UnusableSession
    );
}
#[test]
fn resets_invalidate_settings_but_full_reset_reconfigures_serial() {
    let (mut c, trace) = client_with(
        vec![
            step("ATH1\r", "OK\r>"),
            step("ATZ\r", "ELM327 v2.1\r>"),
            step("ATH1\r", "OK\r>"),
            step("ATRV\r", "LV RESET\r>"),
            step("ATH1\r", "OK\r>"),
            step("ATRV\r", "ELM327 v2.1\r>"),
        ],
        b">".to_vec(),
        Initialization::default(),
        Some(115200),
    );
    connect(&mut c);
    execute(&mut c, Command::Toggle(Toggle::Headers, true)).unwrap();
    execute(&mut c, Command::Fixed(Fixed::Reset)).unwrap();
    assert_eq!(c.settings().headers, None);
    assert_eq!(trace.lock().unwrap().baud_changes, [115200]);
    execute(&mut c, Command::Toggle(Toggle::Headers, true)).unwrap();
    assert_eq!(
        execute(&mut c, Command::Fixed(Fixed::Voltage))
            .unwrap_err()
            .kind,
        ErrorKind::Reset
    );
    assert_eq!(c.settings().headers, None);
    execute(&mut c, Command::Toggle(Toggle::Headers, true)).unwrap();
    assert_eq!(
        execute(&mut c, Command::Fixed(Fixed::Voltage))
            .unwrap_err()
            .kind,
        ErrorKind::Reset
    );
    assert_eq!(c.settings().headers, None);
}
#[test]
fn monitoring_stop_count_and_annotations() {
    let (mut c, trace) = client(vec![
        step("ATMA\r", "ATMA\r7E8 03 41 05 7B\r"),
        step(" ", "STOPPED\r>"),
        step("ATTPA\r", "OK\r>"),
        step("ATMPFECA1\r", "00 FF 00 00 00 00 FF FF\r>"),
    ]);
    connect(&mut c);
    let thread = std::thread::current().id();
    let mut events = 0;
    c.monitor(
        Monitor::All,
        None,
        |r| {
            assert_eq!(std::thread::current().id(), thread);
            assert!(matches!(r, Record::CanFrame { .. }));
            events += 1;
            MonitorControl::Stop
        },
        |r| {
            let s = r.unwrap();
            assert_eq!(s.events, 1);
            assert!(s.stopped_by_caller);
        },
    );
    assert_eq!(events, 1);
    assert_eq!(c.state(), SessionState::Ready);
    c.select_protocol(Protocol::J1939, false, false, None, |r| {
        r.unwrap();
    });
    c.monitor(
        Monitor::Pgn {
            pgn: Pgn::Short(0xFECA),
            count: Some(1),
        },
        None,
        |_| MonitorControl::Continue,
        |r| assert!(!r.unwrap().stopped_by_caller),
    );
    assert_eq!(trace.lock().unwrap().writes, b"ATMA\r ATTPA\rATMPFECA1\r");
    let (mut c, _) = client(vec![step("ATMA\r", "7E8 03 41 05 7B <RX ERROR\r>")]);
    connect(&mut c);
    let mut invalid = false;
    c.monitor(
        Monitor::All,
        None,
        |r| {
            invalid = matches!(r, Record::Annotated { .. });
            MonitorControl::Continue
        },
        |r| assert_eq!(r.unwrap_err().kind, ErrorKind::MalformedResponse),
    );
    assert!(invalid);
}
#[test]
fn baud_handshake_success_and_fallback() {
    let (mut c, trace) = client_with(
        vec![
            step("ATBRD23\r", "OK\rELM327 v2.1\r"),
            step("\r", "OK\r>"),
            step("ATWS\r", "ELM327 v2.1\r>"),
        ],
        b"ELM327 v2.1\r>".to_vec(),
        Initialization::default(),
        Some(38400),
    );
    connect(&mut c);
    let r = execute(&mut c, Command::Byte(ByteSetting::BaudDivisor, 0x23)).unwrap();
    assert_eq!(r.value, ResponseValue::Baud(4_000_000 / 0x23));
    assert_eq!(trace.lock().unwrap().baud_changes, [4_000_000 / 0x23]);
    execute(&mut c, Command::Fixed(Fixed::WarmStart)).unwrap();
    assert_eq!(c.settings().serial_baud, Some(4_000_000 / 0x23));
    let (mut c, trace) = client_with(
        vec![step("ATBRD23\r", "OK\r"), step(" ", "STOPPED\r>")],
        b">".to_vec(),
        Initialization::default(),
        Some(38400),
    );
    connect(&mut c);
    assert!(execute(&mut c, Command::Byte(ByteSetting::BaudDivisor, 0x23)).is_err());
    assert_eq!(
        trace.lock().unwrap().baud_changes,
        [4_000_000 / 0x23, 38400]
    );
    assert_eq!(c.settings().serial_baud, Some(38400));
}
#[test]
fn sleep_unterminated_ok_and_explicit_wake() {
    let trace = Arc::new(Mutex::new(Trace::default()));
    let options = ClientOptions {
        standby_delay: Duration::ZERO,
        ..ClientOptions::default()
    };
    let connector = FakeConnector {
        steps: vec![
            step("ATLP\r", "ATLP\rOK"),
            step(" ", "ELM327 v2.1\r>"),
            step("ATRV\r", "12.4V\r>"),
        ],
        startup: b">".to_vec(),
        trace,
        baud: None,
        max_write: 2,
        fail_open: false,
    };
    let mut c = Client::with_connector(
        TransportConfig::tcp("emulator", 35000),
        options,
        Initialization::default(),
        Box::new(connector),
    )
    .unwrap();
    connect(&mut c);
    assert_eq!(
        execute(&mut c, Command::Fixed(Fixed::LowPower))
            .unwrap()
            .value,
        ResponseValue::Asleep
    );
    assert_eq!(c.state(), SessionState::Asleep);
    assert!(execute(&mut c, Command::Fixed(Fixed::Voltage)).is_err());
    c.wake(Some(Duration::ZERO), |r| {
        assert_eq!(r.unwrap_err().kind, ErrorKind::Timeout)
    });
    c.wake(None, Result::unwrap);
    assert_eq!(c.state(), SessionState::Ready);
    c.read_voltage(None, |r| {
        assert_eq!(r.unwrap().value, ResponseValue::Voltage(12.4))
    });
}
fn obd_setup() -> Vec<Step> {
    vec![
        step("ATDPN\r", "0\r>"),
        step("ATH1\r", "OK\r>"),
        step("ATCAF1\r", "OK\r>"),
        step("ATD0\r", "OK\r>"),
        step("ATS1\r", "OK\r>"),
        step("ATR1\r", "OK\r>"),
        step("ATCEA\r", "OK\r>"),
    ]
}
#[test]
fn high_level_pid_vin_dtc_clear_and_pending_final_response() {
    let mut steps = obd_setup();
    steps.extend([
        step("010C\r","7E8 03 7F 01 78 00 00 00 00\r7E8 04 41 0C 1A F8 00 00 00\r7E9 04 41 0C 00 00 00 00 00\r>"),step("ATDPN\r","6\r>"),
        step("0100\r","7E8 06 41 00 BE 1F B8 10 00\r>"),step("ATDPN\r","6\r>"),
        step("0902\r","7E8 10 14 49 02 01 31 44 34\r7E8 21 47 50 30 30 52 35 35\r7E8 22 42 31 32 33 34 35 36\r>"),step("ATDPN\r","6\r>"),
        step("03\r","7E8 04 43 01 01 33 00 00 00\r>"),step("ATDPN\r","6\r>"),
        step("07\r","7E8 02 47 00 00 00 00 00 00\r>"),step("ATDPN\r","6\r>"),
        step("0A\r","7E8 02 4A 00 00 00 00 00 00\r>"),step("ATDPN\r","6\r>"),
        step("04\r","7E8 01 44 00 00 00 00 00 00\r>"),step("ATDPN\r","6\r>"),
    ]);
    let (mut c, trace) = client(steps);
    connect(&mut c);
    c.read_pid(Pid::EngineRpm, None, |r| {
        let ms = r.unwrap();
        assert_eq!(ms.len(), 2);
        assert_eq!(ms[0].value, 1726.0);
        assert_eq!(ms[1].sender, Sender::Can(0x7E9));
    });
    c.supported_pids(0, None, |r| assert_eq!(r.unwrap()[0].bitmap, 0xBE1FB810));
    c.read_vin(None, |r| {
        assert_eq!(r.unwrap()[0].value, "1D4GP00R55B123456")
    });
    c.read_dtcs(DtcKind::Stored, None, |r| {
        assert_eq!(r.unwrap()[0].codes[0].to_string(), "P0133")
    });
    c.read_dtcs(DtcKind::Pending, None, |r| {
        assert!(r.unwrap()[0].codes.is_empty())
    });
    c.read_dtcs(DtcKind::Permanent, None, |r| {
        assert!(r.unwrap()[0].codes.is_empty())
    });
    assert!(
        !trace
            .lock()
            .unwrap()
            .writes
            .windows(3)
            .any(|w| w == b"04\r")
    );
    c.clear_dtcs(None, |r| assert_eq!(r.unwrap()[0].bytes, [0x44]));
    assert!(trace.lock().unwrap().writes.ends_with(b"04\rATDPN\r"));
}
#[test]
fn discovery_follows_only_advertised_next_block_and_raw_request() {
    let mut steps = obd_setup();
    steps.extend([
        step("0100\r", "7E8 06 41 00 80 00 00 01 00\r>"),
        step("ATDPN\r", "6\r>"),
        step("0120\r", "7E8 06 41 20 80 00 00 00 00\r>"),
        step("ATDPN\r", "6\r>"),
        step("0105\r", "7E8 03 41 05 7B 00 00 00 00\r>"),
        step("ATDPN\r", "6\r>"),
    ]);
    let (mut c, _) = client(steps);
    connect(&mut c);
    c.discover_pids(None, |r| {
        let blocks = r.unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1].base, 0x20);
    });
    c.request_obd(vec![1, 5], None, |r| {
        assert_eq!(r.unwrap()[0].bytes, [0x41, 5, 0x7B])
    });
}
#[test]
fn all_extended_ack_commands_state_and_applicability() {
    let commands = vec![
        Command::CanExtendedAddress(Some(4)),
        Command::Byte(ByteSetting::TesterAddress, 0xF1),
        Command::Timer(TimerMultiplier::Five),
        Command::J1939Timer(TimerMultiplier::One),
        Command::Fixed(Fixed::AllowLong),
        Command::Fixed(Fixed::NormalLength),
        Command::FlowHeader(CanId::Standard(0x7E0)),
        Command::FlowData(vec![0x30, 0, 0]),
        Command::FlowMode(FlowMode::UserHeaderAndData),
        Command::Byte(ByteSetting::BaudTimeout, 0),
        Command::ProtocolB {
            options: 0x82,
            divisor: 2,
        },
        Command::Parameter {
            target: ParameterTarget::One(1),
            action: ParameterAction::Set(0),
        },
        Command::Parameter {
            target: ParameterTarget::One(1),
            action: ParameterAction::Enable,
        },
        Command::Parameter {
            target: ParameterTarget::All,
            action: ParameterAction::Disable,
        },
    ];
    let steps = commands
        .iter()
        .map(|c| Step {
            expected: c.encode(13).unwrap(),
            reply: Reply::Bytes(b"OK\r>".to_vec()),
        })
        .collect();
    let (mut c, _) = client(steps);
    connect(&mut c);
    c.assume_firmware(Firmware::V21);
    for command in commands {
        execute(&mut c, command).unwrap();
    }
    assert_eq!(c.settings().extended_address, Some(Some(4)));
    assert_eq!(c.settings().tester_address, Some(0xF1));
    assert_eq!(c.settings().timer_multiplier, Some(1));
    assert_eq!(c.settings().long_messages, Some(false));
}
#[test]
fn raw_capture_custom_framing_and_description_helper() {
    let trace = Arc::new(Mutex::new(Trace::default()));
    let connector = FakeConnector {
        steps: vec![
            step("ATDP;", "AUTO, ISO 15765-4 CAN;|>"),
            step("ATVENDOR;", "CUSTOM;>"),
        ],
        startup: b">".to_vec(),
        trace,
        baud: None,
        max_write: 2,
        fail_open: false,
    };
    let options = ClientOptions {
        capture_raw: false,
        ..ClientOptions::default()
    };
    let mut c = Client::with_connector(
        TransportConfig::tcp("emulator", 35000),
        options,
        Initialization::default(),
        Box::new(connector),
    )
    .unwrap();
    c.set_framing(b';', b'|').unwrap();
    connect(&mut c);
    c.describe_protocol(None, |r| {
        let r = r.unwrap();
        assert_eq!(r.value, ResponseValue::Text("AUTO, ISO 15765-4 CAN".into()));
        assert!(r.transaction.raw.is_empty());
    });
    execute(&mut c, Command::RawAt("ATVENDOR".into())).unwrap();
    assert_eq!(c.settings().headers, None);
    assert!(c.set_framing(b'>', 10).is_err());
}
