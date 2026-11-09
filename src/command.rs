//! Typed generation of all AT command families in ELM327DSJ.
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Protocol {
    Automatic = 0,
    J1850Pwm = 1,
    J1850Vpw = 2,
    Iso9141 = 3,
    KwpSlow = 4,
    KwpFast = 5,
    Can11_500 = 6,
    Can29_500 = 7,
    Can11_250 = 8,
    Can29_250 = 9,
    J1939 = 10,
    User1 = 11,
    User2 = 12,
}
impl Protocol {
    pub fn from_number(n: u8) -> Result<Self> {
        Ok(match n {
            0 => Self::Automatic,
            1 => Self::J1850Pwm,
            2 => Self::J1850Vpw,
            3 => Self::Iso9141,
            4 => Self::KwpSlow,
            5 => Self::KwpFast,
            6 => Self::Can11_500,
            7 => Self::Can29_500,
            8 => Self::Can11_250,
            9 => Self::Can29_250,
            10 => Self::J1939,
            11 => Self::User1,
            12 => Self::User2,
            _ => return Err(Error::invalid("protocol must be 0..C")),
        })
    }
    pub fn is_can(self) -> bool {
        (self as u8) >= 6
    }
}

/// Commands with no argument. Sleep and monitoring use specialized client paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fixed {
    Defaults,
    ForgetEvents,
    Identify,
    LowPower,
    ReadData,
    WarmStart,
    Reset,
    Description,
    Identifier,
    Voltage,
    Ignition,
    AllowLong,
    NormalLength,
    ActivityCount,
    AutoReceive,
    BufferDump,
    BypassInit,
    DescribeProtocol,
    ProtocolNumber,
    CloseProtocol,
    StandardSearch,
    FastInit,
    SlowInit,
    Keywords,
    RestoreCalibration,
    ResetReceiveFilter,
    RemoteRequest,
    J1939Elm,
    J1939Sae,
    ParameterSummary,
}
impl Fixed {
    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::Defaults => "D",
            Self::ForgetEvents => "FE",
            Self::Identify => "I",
            Self::LowPower => "LP",
            Self::ReadData => "RD",
            Self::WarmStart => "WS",
            Self::Reset => "Z",
            Self::Description => "@1",
            Self::Identifier => "@2",
            Self::Voltage => "RV",
            Self::Ignition => "IGN",
            Self::AllowLong => "AL",
            Self::NormalLength => "NL",
            Self::ActivityCount => "AMC",
            Self::AutoReceive => "AR",
            Self::BufferDump => "BD",
            Self::BypassInit => "BI",
            Self::DescribeProtocol => "DP",
            Self::ProtocolNumber => "DPN",
            Self::CloseProtocol => "PC",
            Self::StandardSearch => "SS",
            Self::FastInit => "FI",
            Self::SlowInit => "SI",
            Self::Keywords => "KW",
            Self::RestoreCalibration => "CV0000",
            Self::ResetReceiveFilter => "CRA",
            Self::RemoteRequest => "RTR",
            Self::J1939Elm => "JE",
            Self::J1939Sae => "JS",
            Self::ParameterSummary => "PPS",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Echo,
    Linefeeds,
    Memory,
    Headers,
    Responses,
    Spaces,
    KeywordCheck,
    CanFormatting,
    FlowControl,
    SilentMonitoring,
    Dlc,
    VariableDlc,
    J1939Headers,
}
impl Toggle {
    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::Echo => "E",
            Self::Linefeeds => "L",
            Self::Memory => "M",
            Self::Headers => "H",
            Self::Responses => "R",
            Self::Spaces => "S",
            Self::KeywordCheck => "KW",
            Self::CanFormatting => "CAF",
            Self::FlowControl => "CFC",
            Self::SilentMonitoring => "CSM",
            Self::Dlc => "D",
            Self::VariableDlc => "V",
            Self::J1939Headers => "JHF",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteSetting {
    BaudDivisor,
    BaudTimeout,
    SaveData,
    ActivityTimeout,
    ReceiveAddress,
    ReceiveAddressAlias,
    ResponseTimeout,
    TesterAddress,
    InitAddress,
    WakeupInterval,
    CanPriority,
}
impl ByteSetting {
    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::BaudDivisor => "BRD",
            Self::BaudTimeout => "BRT",
            Self::SaveData => "SD",
            Self::ActivityTimeout => "AMT",
            Self::ReceiveAddress => "RA",
            Self::ReceiveAddressAlias => "SR",
            Self::ResponseTimeout => "ST",
            Self::TesterAddress => "TA",
            Self::InitAddress => "IIA",
            Self::WakeupInterval => "SW",
            Self::CanPriority => "CP",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdaptiveTiming {
    Off,
    Normal,
    Aggressive,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerMultiplier {
    One,
    Five,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsoBaud {
    Baud10400,
    Baud4800,
    Baud9600,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfrMode {
    Off,
    Automatic,
    Always,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfrSource {
    Header,
    Tester,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowMode {
    Automatic,
    UserHeaderAndData,
    UserData,
}

/// Width is explicit even when the numerical value has leading zeros.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanId {
    Standard(u16),
    Extended(u32),
}
impl CanId {
    pub fn validate(self) -> Result<()> {
        match self {
            Self::Standard(v) if v > 0x7FF => Err(Error::invalid("11-bit ID out of range")),
            Self::Extended(v) if v > 0x1FFFFFFF => Err(Error::invalid("29-bit ID out of range")),
            _ => Ok(()),
        }
    }
    pub(crate) fn encode(self) -> Result<String> {
        self.validate()?;
        Ok(match self {
            Self::Standard(v) => format!("{v:03X}"),
            Self::Extended(v) => format!("{v:08X}"),
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Header {
    Can(CanId),
    ThreeBytes([u8; 3]),
}

/// Explicit byte-width PGN forms preserve MP's four/six-digit syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pgn {
    Short(u16),
    Extended(u32),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Monitor {
    All,
    Receiver(u8),
    Transmitter(u8),
    Dm1,
    Pgn { pgn: Pgn, count: Option<u8> },
}
impl Monitor {
    pub(crate) fn encode(&self) -> Result<String> {
        Ok(match self {
            Self::All => "MA".into(),
            Self::Receiver(v) => format!("MR{v:02X}"),
            Self::Transmitter(v) => format!("MT{v:02X}"),
            Self::Dm1 => "DM1".into(),
            Self::Pgn { pgn, count } => {
                validate_count(*count)?;
                let p = match pgn {
                    Pgn::Short(v) => format!("{v:04X}"),
                    Pgn::Extended(v) if *v <= 0x1FFFF => format!("{v:06X}"),
                    _ => return Err(Error::invalid("MP supports only the Data Page extension")),
                };
                format!(
                    "MP{p}{}",
                    count.map(|v| format!("{v:X}")).unwrap_or_default()
                )
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterTarget {
    One(u8),
    All,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterAction {
    Enable,
    Disable,
    Set(u8),
}

/// One bridge or vehicle operation; encoding never performs I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Fixed(Fixed),
    Toggle(Toggle, bool),
    Byte(ByteSetting, u8),
    Adaptive(AdaptiveTiming),
    Timer(TimerMultiplier),
    J1939Timer(TimerMultiplier),
    IsoBaud(IsoBaud),
    Ifr(IfrMode),
    IfrSource(IfrSource),
    /// Calibration in hundredths of a volt (0000 restores factory calibration).
    Calibrate(u16),
    StoreIdentifier(String),
    Header(Header),
    SetProtocol {
        protocol: Protocol,
        automatic: bool,
        persist: bool,
    },
    EraseProtocol,
    CanExtendedAddress(Option<u8>),
    CanFilter(CanId),
    CanMask(CanId),
    /// Three/eight hex or X nibbles; None restores receive filters.
    CanReceive(Option<String>),
    CanStatus,
    FlowMode(FlowMode),
    FlowHeader(CanId),
    FlowData(Vec<u8>),
    WakeupMessage(Vec<u8>),
    ProtocolB {
        options: u8,
        divisor: u8,
    },
    Parameter {
        target: ParameterTarget,
        action: ParameterAction,
    },
    Monitor(Monitor),
    Vehicle {
        bytes: Vec<u8>,
        responses: Option<u8>,
    },
    /// Prefix must be AT. CR/LF/control characters and prompt injection rejected.
    RawAt(String),
    Repeat,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseExpectation {
    Ack,
    Value,
    Reset,
    Init,
    Bus,
    Stream,
    Baud,
    Sleep,
    Raw,
}
impl Command {
    /// Encode using the caller's configured terminator. Does not validate session
    /// applicability; the client checks firmware, protocol and frame capacity.
    pub fn encode(&self, terminator: u8) -> Result<Vec<u8>> {
        if terminator == 0 || terminator == b'>' {
            return Err(Error::invalid("invalid terminator"));
        }
        let content = match self {
            Self::Fixed(v) => format!("AT{}", v.token()),
            Self::Toggle(t, on) => format!("AT{}{v}", t.token(), v = u8::from(*on)),
            Self::Byte(t, v) => {
                if *t == ByteSetting::BaudDivisor && *v == 0 {
                    return Err(Error::invalid("BRD 00 is invalid"));
                }
                format!("AT{}{v:02X}", t.token())
            }
            Self::Adaptive(v) => format!(
                "ATAT{}",
                match v {
                    AdaptiveTiming::Off => 0,
                    AdaptiveTiming::Normal => 1,
                    AdaptiveTiming::Aggressive => 2,
                }
            ),
            Self::Timer(v) => format!("ATCTM{}", multiplier(*v)),
            Self::J1939Timer(v) => format!("ATJTM{}", multiplier(*v)),
            Self::IsoBaud(v) => format!(
                "ATIB{}",
                match v {
                    IsoBaud::Baud10400 => "10",
                    IsoBaud::Baud4800 => "48",
                    IsoBaud::Baud9600 => "96",
                }
            ),
            Self::Ifr(v) => format!(
                "ATIFR{}",
                match v {
                    IfrMode::Off => 0,
                    IfrMode::Automatic => 1,
                    IfrMode::Always => 2,
                }
            ),
            Self::IfrSource(v) => format!(
                "ATIFR{}",
                match v {
                    IfrSource::Header => "H",
                    IfrSource::Tester => "S",
                }
            ),
            Self::Calibrate(v) => {
                if *v > 9999 {
                    return Err(Error::invalid("calibration requires four decimal digits"));
                }
                format!("ATCV{v:04}")
            }
            Self::StoreIdentifier(v) => {
                if v.len() != 12 || !v.bytes().all(|c| (0x21..=0x5F).contains(&c)) {
                    return Err(Error::invalid(
                        "identifier requires 12 ASCII characters 21..5F",
                    ));
                }
                format!("AT@3{v}")
            }
            Self::Header(h) => format!(
                "ATSH{}",
                match h {
                    Header::Can(id) => id.encode()?,
                    Header::ThreeBytes(b) => hex(b),
                }
            ),
            Self::SetProtocol {
                protocol,
                automatic,
                persist,
            } => format!(
                "AT{}{}{p:X}",
                if *persist { "SP" } else { "TP" },
                if *automatic { "A" } else { "" },
                p = *protocol as u8
            ),
            Self::EraseProtocol => "ATSP00".into(),
            Self::CanExtendedAddress(v) => {
                format!("ATCEA{}", v.map(|b| format!("{b:02X}")).unwrap_or_default())
            }
            Self::CanFilter(id) => format!("ATCF{}", id.encode()?),
            Self::CanMask(id) => format!("ATCM{}", id.encode()?),
            Self::CanReceive(v) => {
                if let Some(s) = v {
                    validate_filter(s)?;
                }
                format!("ATCRA{}", v.as_deref().unwrap_or("").to_ascii_uppercase())
            }
            Self::CanStatus => "ATCS".into(),
            Self::FlowMode(v) => format!(
                "ATFCSM{}",
                match v {
                    FlowMode::Automatic => 0,
                    FlowMode::UserHeaderAndData => 1,
                    FlowMode::UserData => 2,
                }
            ),
            Self::FlowHeader(id) => format!("ATFCSH{}", id.encode()?),
            Self::FlowData(b) => {
                validate_bytes(b, 1, 5)?;
                format!("ATFCSD{}", hex(b))
            }
            Self::WakeupMessage(b) => {
                validate_bytes(b, 1, 6)?;
                format!("ATWM{}", hex(b))
            }
            Self::ProtocolB { options, divisor } => {
                validate_user_options(*options)?;
                if !(1..=0x40).contains(divisor) {
                    return Err(Error::invalid("CAN divisor must be 01..40"));
                }
                format!("ATPB{options:02X}{divisor:02X}")
            }
            Self::Parameter { target, action } => {
                let id = match target {
                    ParameterTarget::One(id) => {
                        validate_parameter(*id, *action)?;
                        *id
                    }
                    ParameterTarget::All => {
                        if matches!(action, ParameterAction::Set(_)) {
                            return Err(Error::invalid("cannot set all PP values at once"));
                        }
                        0xFF
                    }
                };
                format!(
                    "ATPP{id:02X}{}",
                    match action {
                        ParameterAction::Enable => "ON".into(),
                        ParameterAction::Disable => "OFF".into(),
                        ParameterAction::Set(v) => format!("SV{v:02X}"),
                    }
                )
            }
            Self::Monitor(m) => format!("AT{}", m.encode()?),
            Self::Vehicle { bytes, responses } => {
                validate_bytes(bytes, 1, 8)?;
                validate_count(*responses)?;
                format!(
                    "{}{}",
                    hex(bytes),
                    responses.map(|v| format!("{v:X}")).unwrap_or_default()
                )
            }
            Self::RawAt(s) => {
                if !s.is_ascii()
                    || !s.bytes().all(|v| (0x20..=0x7E).contains(&v) && v != b'>')
                    || !s.to_ascii_uppercase().replace(' ', "").starts_with("AT")
                    || s.trim().len() <= 2
                {
                    return Err(Error::invalid("raw AT must be one printable AT command"));
                }
                s.clone()
            }
            Self::Repeat => String::new(),
        };
        if content.as_bytes().contains(&terminator) {
            return Err(Error::invalid("command contains configured terminator"));
        }
        let mut bytes = content.into_bytes();
        bytes.push(terminator);
        Ok(bytes)
    }
    pub fn expectation(&self) -> ResponseExpectation {
        use ResponseExpectation::*;
        match self {
            Self::Monitor(_) => Stream,
            Self::Byte(ByteSetting::BaudDivisor, _) => Baud,
            Self::Fixed(Fixed::LowPower) => Sleep,
            Self::Fixed(Fixed::Reset | Fixed::WarmStart) => Reset,
            Self::Fixed(Fixed::FastInit | Fixed::SlowInit) => Init,
            Self::Vehicle { .. } | Self::Fixed(Fixed::RemoteRequest) => Bus,
            Self::RawAt(_) | Self::Repeat => Raw,
            Self::CanStatus
            | Self::Fixed(
                Fixed::Identify
                | Fixed::ReadData
                | Fixed::Description
                | Fixed::Identifier
                | Fixed::Voltage
                | Fixed::Ignition
                | Fixed::ActivityCount
                | Fixed::BufferDump
                | Fixed::DescribeProtocol
                | Fixed::ProtocolNumber
                | Fixed::Keywords
                | Fixed::ParameterSummary,
            ) => Value,
            _ => Ack,
        }
    }
    /// CTM and new PPs require v2.1; all other typed families exist in v2.0.
    pub fn requires_v21(&self) -> bool {
        matches!(
            self,
            Self::Timer(_)
                | Self::Parameter {
                    target: ParameterTarget::One(0x1E | 0x28),
                    ..
                }
        )
    }
}

pub(crate) fn multiplier(v: TimerMultiplier) -> u8 {
    match v {
        TimerMultiplier::One => 1,
        TimerMultiplier::Five => 5,
    }
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}
pub(crate) fn validate_count(count: Option<u8>) -> Result<()> {
    if count.is_some_and(|n| n == 0 || n > 15) {
        return Err(Error::invalid("response count must be 1..15"));
    }
    Ok(())
}
fn validate_bytes(b: &[u8], min: usize, max: usize) -> Result<()> {
    if !(min..=max).contains(&b.len()) {
        return Err(Error::invalid(format!("expected {min}..{max} bytes")));
    }
    Ok(())
}
fn validate_filter(s: &str) -> Result<()> {
    if !matches!(s.len(), 3 | 8)
        || !s
            .bytes()
            .all(|c| c.is_ascii_hexdigit() || c == b'X' || c == b'x')
    {
        return Err(Error::invalid("CRA requires three/eight hex or X nibbles"));
    }
    if s.len() == 3 && s.as_bytes()[0] != b'X' && s.as_bytes()[0] != b'x' && s.as_bytes()[0] > b'7'
    {
        return Err(Error::invalid("11-bit filter out of range"));
    }
    if s.len() == 8 && s.as_bytes()[0] != b'X' && s.as_bytes()[0] != b'x' && s.as_bytes()[0] > b'1'
    {
        return Err(Error::invalid("29-bit filter out of range"));
    }
    Ok(())
}
fn validate_user_options(v: u8) -> Result<()> {
    if v & 8 != 0 || v & 7 > 2 {
        return Err(Error::invalid("reserved USER CAN options"));
    }
    Ok(())
}
fn validate_parameter(id: u8, action: ParameterAction) -> Result<()> {
    if !matches!(id,0x00..=0x04|0x06|0x07|0x09|0x0A|0x0C..=0x1E|0x21|0x24..=0x26|0x28..=0x2F) {
        return Err(Error::invalid("unimplemented PP"));
    }
    if let ParameterAction::Set(v) = action {
        let valid = match id {
            0x00 | 0x01 | 0x02 | 0x09 | 0x11 | 0x12 | 0x16 | 0x21 | 0x24 | 0x25 | 0x29 => {
                matches!(v, 0 | 0xFF)
            }
            0x04 => v <= 2,
            0x07 => (1..=12).contains(&v),
            0x2B | 0x2D | 0x2F => (1..=0x40).contains(&v),
            0x2C | 0x2E => validate_user_options(v).is_ok(),
            0x0E => v & 1 == 0,
            0x0F => v & 4 != 0,
            0x28 => v & 0x3E == 0x3E,
            _ => true,
        };
        if !valid {
            return Err(Error::invalid(
                "PP value outside documented range/reserved bits",
            ));
        }
    }
    Ok(())
}
