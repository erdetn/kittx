use kittx::{ErrorKind, command::*};

#[test]
fn every_fixed_command_has_documented_encoding_and_expectation() {
    use Fixed::*;
    use ResponseExpectation::*;
    let cases = [
        (Defaults, "D", Ack),
        (ForgetEvents, "FE", Ack),
        (Identify, "I", Value),
        (LowPower, "LP", Sleep),
        (ReadData, "RD", Value),
        (WarmStart, "WS", ResponseExpectation::Reset),
        (Fixed::Reset, "Z", ResponseExpectation::Reset),
        (Description, "@1", Value),
        (Identifier, "@2", Value),
        (Voltage, "RV", Value),
        (Ignition, "IGN", Value),
        (AllowLong, "AL", Ack),
        (NormalLength, "NL", Ack),
        (ActivityCount, "AMC", Value),
        (AutoReceive, "AR", Ack),
        (BufferDump, "BD", Value),
        (BypassInit, "BI", Ack),
        (DescribeProtocol, "DP", Value),
        (ProtocolNumber, "DPN", Value),
        (CloseProtocol, "PC", Ack),
        (StandardSearch, "SS", Ack),
        (FastInit, "FI", Init),
        (SlowInit, "SI", Init),
        (Keywords, "KW", Value),
        (RestoreCalibration, "CV0000", Ack),
        (ResetReceiveFilter, "CRA", Ack),
        (RemoteRequest, "RTR", Bus),
        (J1939Elm, "JE", Ack),
        (J1939Sae, "JS", Ack),
        (ParameterSummary, "PPS", Value),
    ];
    for (fixed, token, expectation) in cases {
        let c = Command::Fixed(fixed);
        assert_eq!(c.encode(b'\r').unwrap(), format!("AT{token}\r").as_bytes());
        assert_eq!(c.expectation(), expectation);
        assert!(!c.requires_v21());
    }
}
#[test]
fn every_toggle_and_byte_setting() {
    let toggles = [
        (Toggle::Echo, "E"),
        (Toggle::Linefeeds, "L"),
        (Toggle::Memory, "M"),
        (Toggle::Headers, "H"),
        (Toggle::Responses, "R"),
        (Toggle::Spaces, "S"),
        (Toggle::KeywordCheck, "KW"),
        (Toggle::CanFormatting, "CAF"),
        (Toggle::FlowControl, "CFC"),
        (Toggle::SilentMonitoring, "CSM"),
        (Toggle::Dlc, "D"),
        (Toggle::VariableDlc, "V"),
        (Toggle::J1939Headers, "JHF"),
    ];
    for (t, s) in toggles {
        for b in [false, true] {
            assert_eq!(
                Command::Toggle(t, b).encode(b'\r').unwrap(),
                format!("AT{s}{}\r", u8::from(b)).as_bytes()
            );
        }
    }
    let bytes = [
        (ByteSetting::BaudDivisor, "BRD"),
        (ByteSetting::BaudTimeout, "BRT"),
        (ByteSetting::SaveData, "SD"),
        (ByteSetting::ActivityTimeout, "AMT"),
        (ByteSetting::ReceiveAddress, "RA"),
        (ByteSetting::ReceiveAddressAlias, "SR"),
        (ByteSetting::ResponseTimeout, "ST"),
        (ByteSetting::TesterAddress, "TA"),
        (ByteSetting::InitAddress, "IIA"),
        (ByteSetting::WakeupInterval, "SW"),
        (ByteSetting::CanPriority, "CP"),
    ];
    for (t, s) in bytes {
        for v in [1, 0xFF] {
            assert_eq!(
                Command::Byte(t, v).encode(b'\r').unwrap(),
                format!("AT{s}{v:02X}\r").as_bytes()
            );
        }
    }
    assert!(
        Command::Byte(ByteSetting::BaudDivisor, 0)
            .encode(b'\r')
            .is_err()
    );
}
#[test]
fn remaining_variants_and_widths() {
    let cases = vec![
        (Command::Adaptive(AdaptiveTiming::Off), "ATAT0"),
        (Command::Adaptive(AdaptiveTiming::Normal), "ATAT1"),
        (Command::Adaptive(AdaptiveTiming::Aggressive), "ATAT2"),
        (Command::Timer(TimerMultiplier::One), "ATCTM1"),
        (Command::Timer(TimerMultiplier::Five), "ATCTM5"),
        (Command::J1939Timer(TimerMultiplier::One), "ATJTM1"),
        (Command::J1939Timer(TimerMultiplier::Five), "ATJTM5"),
        (Command::IsoBaud(IsoBaud::Baud10400), "ATIB10"),
        (Command::IsoBaud(IsoBaud::Baud4800), "ATIB48"),
        (Command::IsoBaud(IsoBaud::Baud9600), "ATIB96"),
        (Command::Ifr(IfrMode::Off), "ATIFR0"),
        (Command::Ifr(IfrMode::Automatic), "ATIFR1"),
        (Command::Ifr(IfrMode::Always), "ATIFR2"),
        (Command::IfrSource(IfrSource::Header), "ATIFRH"),
        (Command::IfrSource(IfrSource::Tester), "ATIFRS"),
        (Command::Calibrate(1247), "ATCV1247"),
        (Command::Calibrate(0), "ATCV0000"),
        (
            Command::StoreIdentifier("MYBOARD_9906".into()),
            "AT@3MYBOARD_9906",
        ),
        (
            Command::Header(Header::Can(CanId::Standard(0x7DF))),
            "ATSH7DF",
        ),
        (
            Command::Header(Header::Can(CanId::Extended(0x18DB33F1))),
            "ATSH18DB33F1",
        ),
        (
            Command::Header(Header::ThreeBytes([0x68, 0x6A, 0xF1])),
            "ATSH686AF1",
        ),
        (
            Command::SetProtocol {
                protocol: Protocol::Can11_500,
                automatic: true,
                persist: true,
            },
            "ATSPA6",
        ),
        (
            Command::SetProtocol {
                protocol: Protocol::KwpSlow,
                automatic: false,
                persist: false,
            },
            "ATTP4",
        ),
        (Command::EraseProtocol, "ATSP00"),
        (Command::CanExtendedAddress(None), "ATCEA"),
        (Command::CanExtendedAddress(Some(4)), "ATCEA04"),
        (Command::CanFilter(CanId::Standard(0x7E8)), "ATCF7E8"),
        (Command::CanFilter(CanId::Extended(1)), "ATCF00000001"),
        (Command::CanMask(CanId::Standard(0x7FF)), "ATCM7FF"),
        (
            Command::CanMask(CanId::Extended(0x1FFFFFFF)),
            "ATCM1FFFFFFF",
        ),
        (Command::CanReceive(None), "ATCRA"),
        (Command::CanReceive(Some("7Ex".into())), "ATCRA7EX"),
        (
            Command::CanReceive(Some("18DAxxF1".into())),
            "ATCRA18DAXXF1",
        ),
        (Command::CanStatus, "ATCS"),
        (Command::FlowMode(FlowMode::Automatic), "ATFCSM0"),
        (Command::FlowMode(FlowMode::UserHeaderAndData), "ATFCSM1"),
        (Command::FlowMode(FlowMode::UserData), "ATFCSM2"),
        (Command::FlowHeader(CanId::Standard(0x7E0)), "ATFCSH7E0"),
        (
            Command::FlowHeader(CanId::Extended(0x18DA10F1)),
            "ATFCSH18DA10F1",
        ),
        (Command::FlowData(vec![0x30, 0, 0]), "ATFCSD300000"),
        (
            Command::WakeupMessage(vec![0xC1, 0x33, 0xF1, 0x3E]),
            "ATWMC133F13E",
        ),
        (
            Command::ProtocolB {
                options: 0x81,
                divisor: 0x0A,
            },
            "ATPB810A",
        ),
        (
            Command::Parameter {
                target: ParameterTarget::One(1),
                action: ParameterAction::Set(0),
            },
            "ATPP01SV00",
        ),
        (
            Command::Parameter {
                target: ParameterTarget::One(1),
                action: ParameterAction::Enable,
            },
            "ATPP01ON",
        ),
        (
            Command::Parameter {
                target: ParameterTarget::One(1),
                action: ParameterAction::Disable,
            },
            "ATPP01OFF",
        ),
        (
            Command::Parameter {
                target: ParameterTarget::All,
                action: ParameterAction::Enable,
            },
            "ATPPFFON",
        ),
        (
            Command::Parameter {
                target: ParameterTarget::All,
                action: ParameterAction::Disable,
            },
            "ATPPFFOFF",
        ),
        (Command::Monitor(Monitor::All), "ATMA"),
        (Command::Monitor(Monitor::Receiver(0xF1)), "ATMRF1"),
        (Command::Monitor(Monitor::Transmitter(0x10)), "ATMT10"),
        (Command::Monitor(Monitor::Dm1), "ATDM1"),
        (
            Command::Monitor(Monitor::Pgn {
                pgn: Pgn::Short(0xFECA),
                count: None,
            }),
            "ATMPFECA",
        ),
        (
            Command::Monitor(Monitor::Pgn {
                pgn: Pgn::Short(0xFECA),
                count: Some(2),
            }),
            "ATMPFECA2",
        ),
        (
            Command::Monitor(Monitor::Pgn {
                pgn: Pgn::Extended(0x1F100),
                count: None,
            }),
            "ATMP01F100",
        ),
        (
            Command::Monitor(Monitor::Pgn {
                pgn: Pgn::Extended(0xFECA),
                count: Some(15),
            }),
            "ATMP00FECAF",
        ),
        (
            Command::Vehicle {
                bytes: vec![1, 12],
                responses: None,
            },
            "010C",
        ),
        (
            Command::Vehicle {
                bytes: vec![1, 12],
                responses: Some(1),
            },
            "010C1",
        ),
        (
            Command::Vehicle {
                bytes: vec![1, 12, 1],
                responses: None,
            },
            "010C01",
        ),
        (Command::RawAt("at vendor 42".into()), "at vendor 42"),
        (Command::Repeat, ""),
    ];
    for (command, text) in cases {
        assert_eq!(
            command.encode(b'\r').unwrap(),
            format!("{text}\r").as_bytes(),
            "{command:?}"
        );
    }
}
#[test]
fn invalid_arguments_are_rejected_without_truncation() {
    let cases = vec![
        Command::Calibrate(10000),
        Command::StoreIdentifier("short".into()),
        Command::StoreIdentifier("abcdefghijkl".into()),
        Command::Header(Header::Can(CanId::Standard(0x800))),
        Command::CanMask(CanId::Extended(0x20000000)),
        Command::FlowData(vec![]),
        Command::FlowData(vec![0; 6]),
        Command::WakeupMessage(vec![]),
        Command::WakeupMessage(vec![0; 7]),
        Command::ProtocolB {
            options: 0x83,
            divisor: 4,
        },
        Command::ProtocolB {
            options: 0,
            divisor: 0,
        },
        Command::ProtocolB {
            options: 0,
            divisor: 0x41,
        },
        Command::CanReceive(Some("8FF".into())),
        Command::CanReceive(Some("FFFFFFFF".into())),
        Command::CanReceive(Some("7EG".into())),
        Command::Vehicle {
            bytes: vec![],
            responses: None,
        },
        Command::Vehicle {
            bytes: vec![0; 9],
            responses: None,
        },
        Command::Vehicle {
            bytes: vec![1],
            responses: Some(0),
        },
        Command::Vehicle {
            bytes: vec![1],
            responses: Some(16),
        },
        Command::Monitor(Monitor::Pgn {
            pgn: Pgn::Extended(0x20000),
            count: None,
        }),
        Command::RawAt("ATZ\rATRV".into()),
        Command::RawAt("AT>".into()),
        Command::RawAt("0100".into()),
        Command::RawAt("AT".into()),
        Command::Parameter {
            target: ParameterTarget::All,
            action: ParameterAction::Set(1),
        },
        Command::Parameter {
            target: ParameterTarget::One(5),
            action: ParameterAction::Enable,
        },
        Command::Parameter {
            target: ParameterTarget::One(4),
            action: ParameterAction::Set(3),
        },
        Command::Parameter {
            target: ParameterTarget::One(0x2D),
            action: ParameterAction::Set(0),
        },
    ];
    for c in cases {
        assert_eq!(
            c.encode(b'\r').unwrap_err().kind,
            ErrorKind::InvalidArgument,
            "{c:?}"
        );
    }
    assert!(Command::Fixed(Fixed::Identify).encode(0).is_err());
    assert!(Command::Fixed(Fixed::Identify).encode(b'I').is_err());
}
#[test]
fn protocol_numbers_and_v21_gate() {
    for n in 0..=12 {
        let p = Protocol::from_number(n).unwrap();
        assert_eq!(p as u8, n);
        assert_eq!(p.is_can(), n >= 6);
    }
    assert!(Protocol::from_number(13).is_err());
    assert!(Command::Timer(TimerMultiplier::Five).requires_v21());
    assert!(!Command::J1939Timer(TimerMultiplier::Five).requires_v21());
    for id in [0x1E, 0x28] {
        assert!(
            Command::Parameter {
                target: ParameterTarget::One(id),
                action: ParameterAction::Enable
            }
            .requires_v21()
        );
    }
    assert!(CanId::Standard(0x7FF).validate().is_ok());
    assert!(CanId::Extended(0x1FFFFFFF).validate().is_ok());
}
