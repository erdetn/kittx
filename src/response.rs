//! Wire records and typed bridge values. Unknown text is retained.
use crate::{
    Error, ErrorKind, Result,
    command::{Command, Fixed, Protocol, ResponseExpectation},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Firmware {
    Unknown,
    V20,
    V21,
}
impl Firmware {
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "ELM327 v2.0" => Self::V20,
            "ELM327 v2.1" => Self::V21,
            _ => Self::Unknown,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Ack,
    Identity(String),
    Hex(Vec<u8>),
    /// CAN header/data presentation with spaces; data includes any PCI/padding.
    CanFrame {
        id: u32,
        dlc: Option<u8>,
        bytes: Vec<u8>,
    },
    Length(usize),
    Segment {
        index: u8,
        digits: usize,
        bytes: Vec<u8>,
    },
    Status(String),
    Alert(String),
    Fault(String),
    Reset(String),
    Annotated {
        line: String,
        annotation: String,
    },
    FlowControl(String),
    RemoteFrame(String),
    Unknown(String),
}
impl Record {
    pub fn parse(line: &str) -> Self {
        let s = line.trim();
        if s == "OK" {
            return Self::Ack;
        }
        if s.starts_with("ELM327 ") {
            return Self::Identity(s.into());
        }
        if s == "LV RESET" || s == "ERR94" {
            return Self::Reset(s.into());
        }
        if matches!(s, "ACT ALERT" | "!ACT ALERT" | "LP ALERT" | "!LP ALERT") {
            return Self::Alert(s.into());
        }
        if s.starts_with("SEARCHING")
            || s.starts_with("BUS INIT:") && !s.contains("ERROR")
            || s == "STOPPED"
        {
            return Self::Status(s.into());
        }
        if s == "?"
            || matches!(
                s,
                "NO DATA"
                    | "UNABLE TO CONNECT"
                    | "BUFFER FULL"
                    | "BUS BUSY"
                    | "BUS ERROR"
                    | "CAN ERROR"
                    | "DATA ERROR"
                    | "FB ERROR"
            )
            || s.starts_with("ERR")
            || s.starts_with("BUS INIT:") && s.contains("ERROR")
        {
            return Self::Fault(s.into());
        }
        for annotation in ["<DATA ERROR", "<RX ERROR"] {
            if let Some(prefix) = s.strip_suffix(annotation) {
                return Self::Annotated {
                    line: prefix.trim().into(),
                    annotation: annotation.into(),
                };
            }
        }
        if s.starts_with("FC:") {
            return Self::FlowControl(s.into());
        }
        if s.contains("RTR") {
            return Self::RemoteFrame(s.into());
        }
        if let Some(frame) = can_frame(s) {
            return frame;
        }
        if let Some((header, _)) = s.split_once(char::is_whitespace) {
            if matches!(header.len(), 3 | 8) && header.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Self::Unknown(s.into());
            }
        }
        if let Some((prefix, data)) = s.split_once(':') {
            if matches!(prefix.len(), 1 | 2) && prefix.bytes().all(|b| b.is_ascii_hexdigit()) {
                if let (Ok(index), Ok(bytes)) = (u8::from_str_radix(prefix, 16), parse_hex(data)) {
                    return Self::Segment {
                        index,
                        digits: prefix.len(),
                        bytes,
                    };
                }
            }
        }
        if s.len() == 3 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
            if let Ok(n) = usize::from_str_radix(s, 16) {
                return Self::Length(n);
            }
        }
        if let Ok(b) = parse_hex(s) {
            return Self::Hex(b);
        }
        Self::Unknown(s.into())
    }
}

fn can_frame(s: &str) -> Option<Record> {
    let (header, rest) = s.split_once(char::is_whitespace)?;
    if !matches!(header.len(), 3 | 8) || !header.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let id = u32::from_str_radix(header, 16).ok()?;
    if id > if header.len() == 3 { 0x7FF } else { 0x1FFFFFFF } {
        return None;
    }
    let mut rest = rest.trim();
    let mut dlc = None;
    if let Some((first, data)) = rest.split_once(char::is_whitespace) {
        if first.len() == 1 {
            let n = u8::from_str_radix(first, 16).ok()?;
            if n > 8 {
                return None;
            }
            dlc = Some(n);
            rest = data.trim();
        }
    }
    let bytes = parse_hex(rest).ok()?;
    if bytes.len() > 8 || dlc.is_some_and(|n| n as usize != bytes.len()) {
        return None;
    }
    Some(Record::CanFrame { id, dlc, bytes })
}

/// One completed wire transaction, with echo removed from lines, not raw bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transaction {
    pub raw: Vec<u8>,
    pub lines: Vec<String>,
    pub records: Vec<Record>,
}
impl Transaction {
    pub(crate) fn add(&mut self, line: String, echo: &str) {
        if line.is_empty() || normalize(&line) == normalize(echo) {
            return;
        }
        self.records.push(Record::parse(&line));
        self.lines.push(line);
    }
    pub fn check(&self) -> Result<()> {
        if self.lines.len() != self.records.len()
            || self
                .lines
                .iter()
                .zip(&self.records)
                .any(|(line, record)| Record::parse(line) != *record)
        {
            let mut error = Error::malformed("transaction line/record mismatch");
            error.partial = self.raw.clone();
            return Err(error);
        }
        for record in &self.records {
            let error = match record {
                Record::Reset(s) => Some(Error::new(ErrorKind::Reset, s)),
                Record::Fault(s) => Some(Error::new(
                    match s.as_str() {
                        "?" => ErrorKind::Rejected,
                        "NO DATA" => ErrorKind::NoData,
                        "BUFFER FULL" => ErrorKind::BufferOverflow,
                        _ => ErrorKind::BridgeFault,
                    },
                    s,
                )),
                Record::Annotated { annotation, .. } => Some(Error::malformed(annotation)),
                _ => None,
            };
            if let Some(mut e) = error {
                e.partial = self.raw.clone();
                return Err(e);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResponseValue {
    Ack,
    Firmware { version: Firmware, identity: String },
    Voltage(f64),
    Byte(u8),
    Ignition(bool),
    Protocol { protocol: Protocol, automatic: bool },
    Buffer { length: u8, bytes: [u8; 12] },
    Parameters(Vec<ParameterValue>),
    Text(String),
    Bus,
    Raw,
    Asleep,
    Baud(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParameterValue {
    pub id: u8,
    pub value: u8,
    pub enabled: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub value: ResponseValue,
    pub transaction: Transaction,
}
impl Response {
    /// Parse a completed bridge transaction with the command's expected schema.
    /// Streaming/baud/sleep framing must first be handled by their client paths.
    pub fn parse(command: &Command, transaction: Transaction) -> Result<Self> {
        decode(command, transaction)
    }
}

pub(crate) fn decode(command: &Command, transaction: Transaction) -> Result<Response> {
    transaction.check()?;
    if matches!(
        command.expectation(),
        ResponseExpectation::Stream | ResponseExpectation::Baud | ResponseExpectation::Sleep
    ) {
        return Err(Error::invalid(
            "special response framing requires the corresponding client operation",
        ));
    }
    if !matches!(
        command,
        Command::Fixed(Fixed::Identify | Fixed::Reset | Fixed::WarmStart)
    ) && transaction
        .records
        .iter()
        .any(|record| matches!(record, Record::Identity(_)))
    {
        let mut error = Error::new(ErrorKind::Reset, "unexpected startup identification");
        error.partial = transaction.raw;
        return Err(error);
    }
    let value=(||{
        use ResponseValue::*;
        let value_lines: Vec<_> = transaction.lines.iter().zip(&transaction.records)
            .filter(|(_, record)| !matches!(record, Record::Status(_) | Record::Alert(_)))
            .map(|(line, _)| line.as_str()).collect();
        let text = value_lines.join("\n");
        let one = || -> Result<&str> {
            if value_lines.len() != 1 { return Err(Error::malformed("expected one value line")); }
            Ok(value_lines[0])
        };
        Ok(match command {
            Command::Fixed(Fixed::Identify|Fixed::Reset|Fixed::WarmStart)=>{
                let identity=transaction.lines.iter().find(|s|s.starts_with("ELM327 ")).ok_or_else(||Error::malformed("missing ELM identification"))?.clone();
                Firmware{version:crate::response::Firmware::parse(&identity),identity}
            },
            Command::Fixed(Fixed::Voltage)=>{let s=one()?;let number=s.strip_suffix('V').ok_or_else(||Error::malformed("voltage suffix missing"))?;let v:f64=number.parse().map_err(|_|Error::malformed("invalid voltage"))?;if !v.is_finite()||v<0.0{return Err(Error::malformed("invalid voltage"));}Voltage(v)},
            Command::Fixed(Fixed::ReadData|Fixed::ActivityCount)=>{let b=parse_hex(one()?)?;if b.len()!=1{return Err(Error::malformed("expected one byte"));}Byte(b[0])},
            Command::Fixed(Fixed::Ignition)=>Ignition(match one()?{"ON"=>true,"OFF"=>false,_=>return Err(Error::malformed("invalid ignition value"))}),
            Command::Fixed(Fixed::ProtocolNumber)=>{let s=one()?;let (automatic,n)=if s.len()==2&&s.starts_with('A'){(true,&s[1..])}else{(false,s)};if n.len()!=1{return Err(Error::malformed("invalid protocol number"));}let n=u8::from_str_radix(n,16).map_err(|_|Error::malformed("invalid protocol number"))?;Protocol{protocol:crate::command::Protocol::from_number(n).map_err(|_|Error::malformed("unknown protocol number"))?,automatic}},
            Command::Fixed(Fixed::BufferDump)=>{let b=parse_hex(one()?)?;if b.len()!=13{return Err(Error::malformed("buffer dump requires 13 bytes"));}let mut bytes=[0;12];bytes.copy_from_slice(&b[1..]);Buffer{length:b[0],bytes}},
            Command::Fixed(Fixed::ParameterSummary)=>Parameters(parse_parameters(&text)?),
            Command::Fixed(Fixed::Identifier)=>{let s=one()?;if s.len()!=12||!s.bytes().all(|b|(0x21..=0x5F).contains(&b)){return Err(Error::malformed("invalid identifier"));}Text(s.into())},
            _=>match command.expectation(){
                ResponseExpectation::Ack|ResponseExpectation::Init=>{if !transaction.records.iter().any(|r|matches!(r,Record::Ack)||matches!(r,Record::Status(s) if s.starts_with("BUS INIT:") && s.ends_with("OK"))){return Err(Error::malformed("missing OK acknowledgment"));}Ack},
                ResponseExpectation::Value=>{if text.is_empty(){return Err(Error::malformed("missing value"));}Text(text)},ResponseExpectation::Bus=>Bus,_=>Raw,
            },
        })
    })().map_err(|mut e:Error|{e.partial=transaction.raw.clone();e})?;
    Ok(Response { value, transaction })
}

pub fn parse_hex(s: &str) -> Result<Vec<u8>> {
    let chars: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if chars.is_empty() || chars.len() % 2 != 0 || !chars.iter().all(u8::is_ascii_hexdigit) {
        return Err(Error::malformed("expected paired hexadecimal bytes"));
    }
    chars
        .chunks_exact(2)
        .map(|pair| {
            let s = std::str::from_utf8(pair).map_err(|_| Error::malformed("invalid hex"))?;
            u8::from_str_radix(s, 16).map_err(|_| Error::malformed("invalid hex"))
        })
        .collect()
}
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_ascii_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}
fn parse_parameters(text: &str) -> Result<Vec<ParameterValue>> {
    if !text.is_ascii() {
        return Err(Error::malformed("PPS must be ASCII"));
    }
    let tokens: Vec<_> = text.split_whitespace().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];
        let (id, value) = token
            .split_once(':')
            .ok_or_else(|| Error::malformed("invalid PPS item"))?;
        let (value, flag) = if value.len() == 3 {
            (&value[..2], &value[2..])
        } else {
            if i + 1 >= tokens.len() {
                return Err(Error::malformed("missing PPS flag"));
            }
            i += 1;
            (value, tokens[i])
        };
        if id.len() != 2 || value.len() != 2 {
            return Err(Error::malformed("invalid PPS width"));
        }
        out.push(ParameterValue {
            id: u8::from_str_radix(id, 16).map_err(|_| Error::malformed("invalid PPS ID"))?,
            value: u8::from_str_radix(value, 16)
                .map_err(|_| Error::malformed("invalid PPS byte"))?,
            enabled: match flag {
                "N" => true,
                "F" => false,
                _ => return Err(Error::malformed("invalid PPS flag")),
            },
        });
        i += 1;
    }
    if out.is_empty() {
        return Err(Error::malformed("empty PPS"));
    }
    Ok(out)
}
