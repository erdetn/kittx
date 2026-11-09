mod common;
use common::*;
use kittx::{
    command::*,
    response::{Record, ResponseValue},
    *,
};
use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
fn run(c: &mut Client, command: Command) -> Result<Response> {
    let mut r = None;
    c.execute(command, None, |v| r = Some(v));
    r.unwrap()
}

#[test]
fn identifier_starting_with_prompt_and_alert_among_value_lines() {
    let (mut c, _) = client(vec![
        step("AT@2\r", "AT@2\r>BCDEFGHIJKL\r>"),
        step("ATRV\r", "ACT ALERT\r12.6V\r>"),
    ]);
    connect(&mut c);
    assert_eq!(
        run(&mut c, Command::Fixed(Fixed::Identifier))
            .unwrap()
            .value,
        ResponseValue::Text(">BCDEFGHIJKL".into())
    );
    let response = run(&mut c, Command::Fixed(Fixed::Voltage)).unwrap();
    assert_eq!(response.value, ResponseValue::Voltage(12.6));
    assert!(matches!(&response.transaction.records[0], Record::Alert(_)));
}
#[test]
fn pp_framing_activation_and_summary_round_trip() {
    let (mut c, trace) = client(vec![
        step("ATPP0DON\r", "OK\r>"),
        step("ATPPS\r", "0D:3B N 0A:0A F\r>"),
        step("ATWS\r", "ELM327 v2.1;>"),
        step("ATRV;", "12.7V;>"),
    ]);
    connect(&mut c);
    run(
        &mut c,
        Command::Parameter {
            target: ParameterTarget::One(0x0D),
            action: ParameterAction::Enable,
        },
    )
    .unwrap();
    assert_eq!(
        run(&mut c, Command::Fixed(Fixed::WarmStart))
            .unwrap_err()
            .kind,
        ErrorKind::UnknownCapability
    );
    run(&mut c, Command::Fixed(Fixed::ParameterSummary)).unwrap();
    run(&mut c, Command::Fixed(Fixed::WarmStart)).unwrap();
    assert_eq!(c.settings().cr, b';');
    c.read_voltage(None, |r| {
        assert_eq!(r.unwrap().value, ResponseValue::Voltage(12.7))
    });
    assert!(trace.lock().unwrap().writes.ends_with(b"ATWS\rATRV;"));
}
#[test]
fn user2_j1939_monitor_and_all_monitor_modes() {
    let commands = vec![
        Command::Parameter {
            target: ParameterTarget::One(0x2E),
            action: ParameterAction::Set(0x82),
        },
        Command::Parameter {
            target: ParameterTarget::One(0x2E),
            action: ParameterAction::Enable,
        },
        Command::Fixed(Fixed::WarmStart),
        Command::SetProtocol {
            protocol: Protocol::User2,
            automatic: false,
            persist: false,
        },
    ];
    let mut steps: Vec<_> = commands
        .iter()
        .map(|command| Step {
            expected: command.encode(13).unwrap(),
            reply: Reply::Bytes(if *command == Command::Fixed(Fixed::WarmStart) {
                b"ELM327 v2.1\r>".to_vec()
            } else {
                b"OK\r>".to_vec()
            }),
        })
        .collect();
    steps.extend([
        step("ATDM1\r", "00 FF 00 00 00 00 FF FF\r>"),
        step("ATMRF1\r", "01 02\r>"),
        step("ATMT10\r", "01 02\r>"),
        step("ATMP00FECA1\r", "00 FF 00 00 00 00 FF FF\r>"),
    ]);
    let (mut c, _) = client(steps);
    connect(&mut c);
    for command in commands {
        run(&mut c, command).unwrap();
    }
    for mode in [
        Monitor::Dm1,
        Monitor::Receiver(0xF1),
        Monitor::Transmitter(0x10),
        Monitor::Pgn {
            pgn: Pgn::Extended(0xFECA),
            count: Some(1),
        },
    ] {
        c.monitor(
            mode,
            None,
            |_| MonitorControl::Continue,
            |r| assert_eq!(r.unwrap().events, 1),
        );
    }
}
#[test]
fn v20_jtm_is_separate_and_emulator_override_survives_identification() {
    let (mut c, _) = client_with(
        vec![step("ATI\r", "ELM327 v2.0\r>"), step("ATJTM5\r", "OK\r>")],
        b">".to_vec(),
        Initialization::default(),
        None,
    );
    connect(&mut c);
    c.identify(None, |r| {
        r.unwrap();
    });
    run(&mut c, Command::J1939Timer(TimerMultiplier::Five)).unwrap();
    assert_eq!(c.settings().j1939_timer_multiplier, Some(5));
    assert_eq!(c.settings().timer_multiplier, None);
    assert_eq!(
        run(&mut c, Command::Timer(TimerMultiplier::Five))
            .unwrap_err()
            .kind,
        ErrorKind::UnsupportedFirmware
    );
    let (mut c, _) = client_with(
        vec![
            step("ATI\r", "ELM327 emulator\r>"),
            step("ATCTM5\r", "OK\r>"),
        ],
        b">".to_vec(),
        Initialization::new(vec![
            Command::Fixed(Fixed::Identify),
            Command::Timer(TimerMultiplier::Five),
        ]),
        None,
    );
    c.assume_firmware(Firmware::V21);
    connect(&mut c);
    assert_eq!(c.firmware(), Firmware::V21);
    c.assume_firmware(Firmware::Unknown);
    assert_eq!(c.firmware(), Firmware::Unknown);
}
#[test]
fn negative_ecu_reply_retains_complete_raw_transaction() {
    let mut steps = vec![
        step("ATDPN\r", "6\r>"),
        step("ATH1\r", "OK\r>"),
        step("ATCAF1\r", "OK\r>"),
        step("ATD0\r", "OK\r>"),
        step("ATS1\r", "OK\r>"),
        step("ATR1\r", "OK\r>"),
        step("ATCEA\r", "OK\r>"),
    ];
    steps.extend([
        step(
            "0105\r",
            "7E8 03 41 05 7B 00 00 00 00\r7E9 03 7F 01 12 00 00 00 00\r>",
        ),
        step("ATDPN\r", "6\r>"),
    ]);
    let (mut c, _) = client(steps);
    connect(&mut c);
    c.read_pid(kittx::obd::Pid::CoolantTemperature, None, |r| {
        let e = r.unwrap_err();
        assert_eq!(
            e.kind,
            ErrorKind::EcuNegative {
                service: 1,
                code: 0x12
            }
        );
        let raw = String::from_utf8(e.partial).unwrap();
        assert!(raw.contains("7E8"));
        assert!(raw.contains("7E9"));
    });
    assert_eq!(c.state(), SessionState::Ready);
}
#[test]
fn pending_without_final_reply_and_wrong_protocol_are_explicit() {
    let profile = Initialization::new(vec![
        Command::SetProtocol {
            protocol: Protocol::Can11_500,
            automatic: false,
            persist: false,
        },
        Command::Toggle(Toggle::Headers, true),
        Command::Toggle(Toggle::CanFormatting, true),
        Command::Toggle(Toggle::Dlc, false),
        Command::Toggle(Toggle::Spaces, true),
        Command::Toggle(Toggle::Responses, true),
        Command::CanExtendedAddress(None),
    ]);
    let mut steps: Vec<_> = profile
        .steps
        .iter()
        .map(|command| Step {
            expected: command.encode(13).unwrap(),
            reply: Reply::Bytes(b"OK\r>".to_vec()),
        })
        .collect();
    steps.extend([
        step("0105\r", "7E8 03 7F 01 78 00 00 00 00\r>"),
        step("ATDPN\r", "6\r>"),
    ]);
    let (mut c, _) = client_with(steps, b">".to_vec(), profile, None);
    connect(&mut c);
    c.read_pid(kittx::obd::Pid::CoolantTemperature, None, |r| {
        assert_eq!(r.unwrap_err().kind, ErrorKind::IncompleteMessage)
    });
    let (mut c, trace) = client(vec![step("ATDPN\r", "A\r>")]);
    connect(&mut c);
    c.clear_dtcs(None, |r| {
        assert_eq!(r.unwrap_err().kind, ErrorKind::InvalidArgument)
    });
    assert_eq!(trace.lock().unwrap().writes, b"ATDPN\r");
}
#[test]
fn stream_error_timeout_and_low_power_terminal_callbacks() {
    let (mut c, _) = client(vec![
        step("ATMA\r", "BUFFER FULL\r>"),
        step("ATMA\r", ""),
        step(" ", "STOPPED\r>"),
        step("ATMA\r", "!LP ALERT\r"),
    ]);
    connect(&mut c);
    c.monitor(
        Monitor::All,
        None,
        |_| MonitorControl::Continue,
        |r| assert_eq!(r.unwrap_err().kind, ErrorKind::BufferOverflow),
    );
    c.monitor(
        Monitor::All,
        None,
        |_| MonitorControl::Continue,
        |r| assert_eq!(r.unwrap_err().kind, ErrorKind::Timeout),
    );
    c.monitor(
        Monitor::All,
        None,
        |_| MonitorControl::Continue,
        |r| assert_eq!(r.unwrap_err().kind, ErrorKind::UnusableSession),
    );
    assert_eq!(c.state(), SessionState::Asleep);
}
#[test]
fn io_failure_open_cleanup_and_unknown_previous_command() {
    let trace = Arc::new(Mutex::new(Trace::default()));
    let connector = FakeConnector {
        steps: vec![],
        startup: vec![],
        trace: trace.clone(),
        baud: None,
        max_write: 1,
        fail_open: true,
    };
    let mut c = Client::with_connector(
        TransportConfig::tcp("emulator", 35000),
        ClientOptions::default(),
        Initialization::default(),
        Box::new(connector),
    )
    .unwrap();
    c.connect(None, |r| {
        assert!(matches!(
            r.unwrap_err().kind,
            ErrorKind::Io(io::ErrorKind::ConnectionRefused)
        ))
    });
    assert_eq!(c.state(), SessionState::Disconnected);
    let (mut c, _) = client(vec![step("ATRV\r", "NO DATA\r>")]);
    connect(&mut c);
    assert_eq!(
        run(&mut c, Command::Repeat).unwrap_err().kind,
        ErrorKind::InvalidArgument
    );
    run(&mut c, Command::Fixed(Fixed::Voltage)).unwrap_err();
    assert_eq!(
        run(&mut c, Command::Repeat).unwrap_err().kind,
        ErrorKind::InvalidArgument
    );
}
#[test]
fn frame_capacity_and_rtr_requirements() {
    let (mut c, trace) = client(vec![
        step("ATTP6\r", "OK\r>"),
        step("ATBI\r", "OK\r>"),
        step("ATRTR\r", "7E8 03 41 05 7B\r>"),
        step("ATCEA04\r", "OK\r>"),
        step("ATCAF0\r", "OK\r>"),
        step("ATR0\r", "OK\r>"),
        step("00010203040506\r", ">"),
    ]);
    connect(&mut c);
    run(
        &mut c,
        Command::SetProtocol {
            protocol: Protocol::Can11_500,
            automatic: false,
            persist: false,
        },
    )
    .unwrap();
    assert_eq!(
        run(&mut c, Command::Fixed(Fixed::RemoteRequest))
            .unwrap_err()
            .kind,
        ErrorKind::UnknownCapability
    );
    run(&mut c, Command::Fixed(Fixed::BypassInit)).unwrap();
    run(&mut c, Command::Fixed(Fixed::RemoteRequest)).unwrap();
    run(&mut c, Command::CanExtendedAddress(Some(4))).unwrap();
    assert!(
        run(
            &mut c,
            Command::Vehicle {
                bytes: vec![0; 7],
                responses: None
            }
        )
        .is_err()
    );
    run(&mut c, Command::Toggle(Toggle::CanFormatting, false)).unwrap();
    run(&mut c, Command::Toggle(Toggle::Responses, false)).unwrap();
    run(
        &mut c,
        Command::Vehicle {
            bytes: vec![0, 1, 2, 3, 4, 5, 6],
            responses: None,
        },
    )
    .unwrap();
    assert!(trace.lock().unwrap().writes.ends_with(b"00010203040506\r"));
}
#[test]
fn callback_panic_leaves_completed_session_state() {
    let (mut c, _) = client(vec![step("ATE0\r", "OK\r>")]);
    connect(&mut c);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        c.execute(
            Command::Toggle(Toggle::Echo, false),
            Some(Duration::from_secs(1)),
            |_| panic!("caller panic"),
        )
    }));
    assert!(caught.is_err());
    assert_eq!(c.state(), SessionState::Ready);
    assert_eq!(c.settings().echo, Some(false));
}
