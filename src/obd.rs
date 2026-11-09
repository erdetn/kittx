//! Sender-aware OBD transport reassembly and selected application decoders.
use crate::{
    Error, ErrorKind, Result,
    command::Protocol,
    response::{Record, Transaction, parse_hex},
};
use std::{collections::BTreeMap, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sender {
    Unknown,
    Can(u32),
    Legacy(u8),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcuMessage {
    pub sender: Sender,
    pub bytes: Vec<u8>,
}

/// Explicit wire layout; never guessed from the apparent data length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// CAF1, H0: application bytes or length/colon segments, no sender identity.
    Formatted,
    /// CAN H1, PCI visible; extended_address selects TA's receive address.
    Can {
        extended_id: bool,
        dlc: bool,
        extended_address: Option<u8>,
    },
    /// Complete legacy header and checksum; KWP may have a fourth length byte.
    Legacy(Protocol),
}
struct Assembly {
    length: usize,
    bytes: Vec<u8>,
    next: u8,
}

/// Reassemble a completed transaction. Padding outside declared length is ignored.
/// Multiple interleaved formatted H0 responses are rejected as ambiguous.
pub fn reassemble(transaction: &Transaction, layout: Layout) -> Result<Vec<EcuMessage>> {
    let result = (|| {
        if let Layout::Legacy(p) = layout {
            if !matches!(
                p,
                Protocol::J1850Pwm
                    | Protocol::J1850Vpw
                    | Protocol::Iso9141
                    | Protocol::KwpSlow
                    | Protocol::KwpFast
            ) {
                return Err(Error::invalid("legacy layout requires protocol 1..5"));
            }
        }
        if transaction.lines.len() != transaction.records.len() {
            return Err(Error::malformed("transaction line/record mismatch"));
        }
        transaction.check()?;
        let mut messages = Vec::new();
        let mut active: BTreeMap<Sender, Assembly> = BTreeMap::new();
        let mut formatted: Option<Assembly> = None;
        for (line, record) in transaction.lines.iter().zip(&transaction.records) {
            match layout {
                Layout::Formatted => match record {
                    Record::Hex(bytes) => messages.push(EcuMessage {
                        sender: Sender::Unknown,
                        bytes: bytes.clone(),
                    }),
                    Record::Length(length) => {
                        if formatted.is_some() {
                            return Err(Error::new(
                                ErrorKind::AmbiguousSender,
                                "overlapping H0 multiframe replies",
                            ));
                        }
                        if *length == 0 || *length > 4095 {
                            return Err(Error::malformed("invalid multiframe length"));
                        }
                        formatted = Some(Assembly {
                            length: *length,
                            bytes: Vec::new(),
                            next: 0,
                        });
                    }
                    Record::Segment {
                        index,
                        digits,
                        bytes,
                    } => {
                        let a = formatted.as_mut().ok_or_else(|| {
                            Error::new(ErrorKind::IncompleteMessage, "segment without length")
                        })?;
                        if *digits != 1 {
                            return Err(Error::new(
                                ErrorKind::UnsupportedFirmware,
                                "J1939 application decoding is deferred",
                            ));
                        }
                        if *index != a.next {
                            return Err(Error::new(
                                ErrorKind::AmbiguousSender,
                                "H0 segment sequence is missing, duplicate or interleaved",
                            ));
                        }
                        a.next = (a.next + 1) & 15;
                        a.bytes.extend(bytes);
                        if a.bytes.len() >= a.length {
                            let mut a = formatted.take().expect("present assembly");
                            a.bytes.truncate(a.length);
                            messages.push(EcuMessage {
                                sender: Sender::Unknown,
                                bytes: a.bytes,
                            });
                        }
                    }
                    Record::Unknown(_) => return Err(Error::malformed("unknown OBD data line")),
                    _ => {}
                },
                Layout::Can {
                    extended_id,
                    dlc,
                    extended_address,
                } => {
                    if matches!(
                        record,
                        Record::Status(_)
                            | Record::Alert(_)
                            | Record::FlowControl(_)
                            | Record::RemoteFrame(_)
                            | Record::Ack
                    ) {
                        continue;
                    }
                    let compact: String =
                        line.chars().filter(|c| !c.is_ascii_whitespace()).collect();
                    let width = if extended_id { 8 } else { 3 };
                    if !compact.is_ascii()
                        || compact.len() <= width
                        || !compact.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        return Err(Error::malformed("invalid CAN line"));
                    }
                    let id = u32::from_str_radix(&compact[..width], 16)
                        .map_err(|_| Error::malformed("invalid CAN ID"))?;
                    if id > if extended_id { 0x1FFFFFFF } else { 0x7FF } {
                        return Err(Error::malformed("CAN ID out of range"));
                    }
                    let mut offset = width;
                    let declared = if dlc {
                        let n = u8::from_str_radix(&compact[offset..offset + 1], 16)
                            .map_err(|_| Error::malformed("invalid DLC"))?;
                        if n > 8 {
                            return Err(Error::malformed("DLC exceeds 8"));
                        }
                        offset += 1;
                        Some(n as usize)
                    } else {
                        None
                    };
                    let mut data = parse_hex(&compact[offset..])?;
                    if data.len() > 8 || declared.is_some_and(|n| n != data.len()) {
                        return Err(Error::malformed("CAN data length mismatch"));
                    }
                    if let Some(address) = extended_address {
                        if data.first() != Some(&address) {
                            return Err(Error::malformed("unexpected extended address"));
                        }
                        data.remove(0);
                    }
                    let pci = *data
                        .first()
                        .ok_or_else(|| Error::malformed("missing PCI"))?;
                    let sender = Sender::Can(id);
                    match pci >> 4 {
                        0 => {
                            let len = (pci & 15) as usize;
                            if data.len() < len + 1 {
                                return Err(Error::malformed("invalid single frame length"));
                            }
                            if active.contains_key(&sender) {
                                return Err(Error::new(
                                    ErrorKind::IncompleteMessage,
                                    "single frame interrupts assembly",
                                ));
                            }
                            messages.push(EcuMessage {
                                sender,
                                bytes: data[1..1 + len].to_vec(),
                            });
                        }
                        1 => {
                            if data.len() < 3 {
                                return Err(Error::malformed("short first frame"));
                            }
                            let length = (((pci & 15) as usize) << 8) | data[1] as usize;
                            if length <= data.len() - 2 {
                                return Err(Error::malformed("unnecessary first frame"));
                            }
                            if active
                                .insert(
                                    sender,
                                    Assembly {
                                        length,
                                        bytes: data[2..].to_vec(),
                                        next: 1,
                                    },
                                )
                                .is_some()
                            {
                                return Err(Error::new(
                                    ErrorKind::IncompleteMessage,
                                    "duplicate first frame",
                                ));
                            }
                        }
                        2 => {
                            let a = active.get_mut(&sender).ok_or_else(|| {
                                Error::new(
                                    ErrorKind::IncompleteMessage,
                                    "consecutive frame without first frame",
                                )
                            })?;
                            if pci & 15 != a.next {
                                return Err(Error::new(
                                    ErrorKind::IncompleteMessage,
                                    "consecutive frame sequence mismatch",
                                ));
                            }
                            if data.len() < 2 {
                                return Err(Error::malformed("empty consecutive frame"));
                            }
                            a.next = (a.next + 1) & 15;
                            a.bytes.extend_from_slice(&data[1..]);
                            if a.bytes.len() >= a.length {
                                let mut a = active.remove(&sender).expect("present assembly");
                                a.bytes.truncate(a.length);
                                messages.push(EcuMessage {
                                    sender,
                                    bytes: a.bytes,
                                });
                            }
                        }
                        3 => {}
                        _ => return Err(Error::malformed("unsupported CAN PCI")),
                    }
                }
                Layout::Legacy(protocol) => {
                    if matches!(record, Record::Status(_) | Record::Alert(_) | Record::Ack) {
                        continue;
                    }
                    let b = parse_hex(line)?;
                    if b.len() < 5 {
                        return Err(Error::malformed("legacy message too short"));
                    }
                    let checksum = *b.last().expect("nonempty");
                    let expected = if matches!(protocol, Protocol::J1850Pwm | Protocol::J1850Vpw) {
                        crc_j1850(&b[..b.len() - 1])
                    } else {
                        b[..b.len() - 1].iter().fold(0u8, |a, b| a.wrapping_add(*b))
                    };
                    if checksum != expected {
                        return Err(Error::malformed("legacy checksum mismatch"));
                    }
                    let start = if matches!(protocol, Protocol::KwpSlow | Protocol::KwpFast)
                        && b[0] & 0x3F == 0
                    {
                        4
                    } else {
                        3
                    };
                    if b.len() <= start + 1 {
                        return Err(Error::malformed("empty legacy payload"));
                    }
                    if matches!(protocol, Protocol::KwpSlow | Protocol::KwpFast) {
                        let len = if start == 4 {
                            b[3] as usize
                        } else {
                            (b[0] & 0x3F) as usize
                        };
                        if len != b.len() - start - 1 {
                            return Err(Error::malformed("KWP length mismatch"));
                        }
                    }
                    messages.push(EcuMessage {
                        sender: Sender::Legacy(b[2]),
                        bytes: b[start..b.len() - 1].to_vec(),
                    });
                }
            }
        }
        if formatted.is_some() || !active.is_empty() {
            return Err(Error::new(
                ErrorKind::IncompleteMessage,
                "incomplete multiframe response",
            ));
        }
        if messages.is_empty() {
            return Err(Error::new(ErrorKind::NoData, "no ECU messages"));
        }
        Ok(messages)
    })();
    result.map_err(|mut e: Error| {
        e.partial = transaction.raw.clone();
        e
    })
}
fn crc_j1850(b: &[u8]) -> u8 {
    let mut crc = 0xFFu8;
    for byte in b {
        crc ^= *byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x1D
            } else {
                crc << 1
            };
        }
    }
    crc ^ 0xFF
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Pid {
    EngineLoad = 0x04,
    CoolantTemperature = 0x05,
    ManifoldPressure = 0x0B,
    EngineRpm = 0x0C,
    VehicleSpeed = 0x0D,
    IntakeTemperature = 0x0F,
    ThrottlePosition = 0x11,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Percent,
    Celsius,
    Kilopascals,
    Rpm,
    KilometresPerHour,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Measurement {
    pub sender: Sender,
    pub pid: Pid,
    pub value: f64,
    pub unit: Unit,
}
impl Pid {
    pub fn decode(self, message: &EcuMessage) -> Result<Measurement> {
        positive(message, 1)?;
        let b = &message.bytes;
        let length = if self == Self::EngineRpm { 4 } else { 3 };
        if b.len() != length || b[1] != self as u8 {
            return Err(Error::malformed("PID response/length mismatch"));
        }
        let a = b[2] as f64;
        let (value, unit) = match self {
            Self::EngineLoad | Self::ThrottlePosition => (a * 100.0 / 255.0, Unit::Percent),
            Self::CoolantTemperature | Self::IntakeTemperature => (a - 40.0, Unit::Celsius),
            Self::ManifoldPressure => (a, Unit::Kilopascals),
            Self::EngineRpm => ((a * 256.0 + b[3] as f64) / 4.0, Unit::Rpm),
            Self::VehicleSpeed => (a, Unit::KilometresPerHour),
        };
        Ok(Measurement {
            sender: message.sender,
            pid: self,
            value,
            unit,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportedPids {
    pub sender: Sender,
    pub base: u8,
    pub bitmap: u32,
}
impl SupportedPids {
    pub fn decode(message: &EcuMessage, base: u8) -> Result<Self> {
        if base % 32 != 0 || base > 0xE0 {
            return Err(Error::invalid("PID support base must be 00,20,..E0"));
        }
        positive(message, 1)?;
        let b = &message.bytes;
        if b.len() != 6 || b[1] != base {
            return Err(Error::malformed("invalid support bitmap"));
        }
        Ok(Self {
            sender: message.sender,
            base,
            bitmap: u32::from_be_bytes([b[2], b[3], b[4], b[5]]),
        })
    }
    pub fn supports(&self, pid: u8) -> bool {
        let distance = u16::from(pid).wrapping_sub(u16::from(self.base));
        (1..=32).contains(&distance) && self.bitmap & (1u32 << (32 - distance)) != 0
    }
    pub fn has_next_block(&self) -> bool {
        self.base < 0xE0 && self.bitmap & 1 != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtcKind {
    Stored,
    Pending,
    Permanent,
}
impl DtcKind {
    pub fn service(self) -> u8 {
        match self {
            Self::Stored => 3,
            Self::Pending => 7,
            Self::Permanent => 10,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtcSystem {
    Powertrain,
    Chassis,
    Body,
    Network,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dtc {
    pub system: DtcSystem,
    pub number: u16,
}
impl Dtc {
    pub fn from_bytes(bytes: [u8; 2]) -> Self {
        let raw = u16::from_be_bytes(bytes);
        Self {
            system: match raw >> 14 {
                0 => DtcSystem::Powertrain,
                1 => DtcSystem::Chassis,
                2 => DtcSystem::Body,
                _ => DtcSystem::Network,
            },
            number: raw & 0x3FFF,
        }
    }
}
impl fmt::Display for Dtc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{n:04X}",
            match self.system {
                DtcSystem::Powertrain => 'P',
                DtcSystem::Chassis => 'C',
                DtcSystem::Body => 'B',
                DtcSystem::Network => 'U',
            },
            n = self.number
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TroubleCodes {
    pub sender: Sender,
    pub kind: DtcKind,
    pub codes: Vec<Dtc>,
}
pub fn decode_dtcs(message: &EcuMessage, kind: DtcKind, can: bool) -> Result<TroubleCodes> {
    positive(message, kind.service())?;
    let b = &message.bytes;
    let start = if can { 2 } else { 1 };
    if b.len() < start {
        return Err(Error::malformed("DTC count missing"));
    }
    let payload = &b[start..];
    if payload.len() % 2 != 0 {
        return Err(Error::malformed("odd DTC payload length"));
    }
    let count = if can {
        b[1] as usize
    } else {
        payload.len() / 2
    };
    if count > payload.len() / 2 {
        return Err(Error::malformed("DTC count exceeds payload"));
    }
    if can && payload[count * 2..].iter().any(|b| *b != 0) {
        return Err(Error::malformed("nonzero DTC padding"));
    }
    let codes = payload[..count * 2]
        .chunks_exact(2)
        .filter(|b| b != &[0, 0])
        .map(|b| Dtc::from_bytes([b[0], b[1]]))
        .collect();
    Ok(TroubleCodes {
        sender: message.sender,
        kind,
        codes,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vin {
    pub sender: Sender,
    pub value: String,
}
/// Group legacy application segments by sender; CAN messages are already assembled.
pub fn decode_vins(messages: &[EcuMessage], can: bool) -> Result<Vec<Vin>> {
    let mut groups: BTreeMap<Sender, Vec<&EcuMessage>> = BTreeMap::new();
    for m in messages {
        groups.entry(m.sender).or_default().push(m);
    }
    let mut out = Vec::new();
    for (sender, mut ms) in groups {
        for m in &ms {
            positive(m, 9)?;
            if m.bytes.len() < 3 || m.bytes[1] != 2 {
                return Err(Error::malformed("not a VIN response"));
            }
        }
        let data = if can {
            if ms.len() != 1 {
                return Err(Error::new(
                    ErrorKind::AmbiguousSender,
                    "multiple VINs without unique sender",
                ));
            }
            if ms[0].bytes[2] != 1 {
                return Err(Error::malformed("invalid VIN item count"));
            }
            ms[0].bytes[3..].to_vec()
        } else {
            ms.sort_by_key(|m| m.bytes[2]);
            let mut data = Vec::new();
            for (i, m) in ms.iter().enumerate() {
                if m.bytes[2] as usize != i + 1 || m.bytes.len() != 7 {
                    return Err(Error::new(
                        ErrorKind::IncompleteMessage,
                        "invalid legacy VIN segments",
                    ));
                }
                data.extend_from_slice(&m.bytes[3..]);
            }
            let first = data.iter().position(|b| *b != 0).unwrap_or(data.len());
            data[first..].to_vec()
        };
        if data.len() != 17
            || !data.iter().all(|b| {
                b.is_ascii_alphanumeric()
                    && !matches!(*b, b'I' | b'O' | b'Q')
                    && !b.is_ascii_lowercase()
            })
        {
            return Err(Error::malformed(
                "VIN must contain 17 valid ASCII VIN characters",
            ));
        }
        out.push(Vin {
            sender,
            value: String::from_utf8(data).map_err(|_| Error::malformed("invalid VIN"))?,
        });
    }
    if out.is_empty() {
        return Err(Error::new(ErrorKind::NoData, "no VIN replies"));
    }
    Ok(out)
}

/// Validate service response, preserving ECU negative-response semantics.
pub fn positive(message: &EcuMessage, service: u8) -> Result<()> {
    let expected = service
        .checked_add(0x40)
        .ok_or_else(|| Error::invalid("service has no OBD positive-response encoding"))?;
    let b = &message.bytes;
    if b.first() == Some(&0x7F) {
        if b.len() != 3 {
            return Err(Error::malformed("invalid negative response length"));
        }
        return Err(Error::new(
            ErrorKind::EcuNegative {
                service: b[1],
                code: b[2],
            },
            "ECU negative response",
        ));
    }
    if b.first() != Some(&expected) {
        return Err(Error::malformed("unexpected response service"));
    }
    Ok(())
}
