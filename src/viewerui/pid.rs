//! Read-only Service 01 numeric interpretation. Original recording bytes stay intact.
//! Reference and data license: `pids/README.md` and `pids/LICENSE-OBDb`.
use std::sync::OnceLock;

/// One physical measurement/counter in an ECU PID block. Bit offsets start at the
/// most significant bit of data byte A, after the response service and PID bytes.
#[derive(Debug)]
pub struct Field {
    pub pid: u8,
    pub key: &'static str,
    pub name: &'static str,
    pub unit: &'static str,
    bit: usize,
    width: usize,
    signed: bool,
    multiplier: f64,
    divisor: f64,
    offset: f64,
    null_min: Option<f64>,
    null_max: Option<f64>,
    support: Vec<usize>,
    unavailable: Vec<usize>,
    wide: Option<usize>,
}

impl Field {
    /// Stable viewer-only identity. This is not a stored measurement identifier.
    pub fn signal(&self) -> String {
        format!("01:{:02X}:decoded:{}", self.pid, self.key)
    }

    /// Original raw recording series; several fields may share this source.
    pub fn source_signal(&self) -> String {
        format!("01:{:02X}:raw", self.pid)
    }

    /// Decode only matching positive responses. Missing fields, unsupported sensors,
    /// no-result flags and documented unavailable encodings produce a gap, never zero.
    /// A truncated block can still supply complete earlier fields.
    pub fn decode(&self, payload: &[u8]) -> Option<f64> {
        if payload.get(..2)? != [0x41, self.pid] {
            return None;
        }
        let bytes = &payload[2..];
        if !self.support.is_empty()
            && !self
                .support
                .iter()
                .any(|&bit| bits(bytes, bit, 1) == Some(1))
        {
            return None;
        }
        if self
            .unavailable
            .iter()
            .any(|&bit| bits(bytes, bit, 1) != Some(0))
        {
            return None;
        }
        let raw = bits(bytes, self.bit, self.width)?;
        let integer = if self.signed && raw & (1 << (self.width - 1)) != 0 {
            i64::from(raw) - (1i64 << self.width)
        } else {
            i64::from(raw)
        };
        let wide = self.wide.is_some_and(|bit| bits(bytes, bit, 1) == Some(1));
        let value = integer as f64 * self.multiplier * if wide { 8. } else { 1. } / self.divisor
            + self.offset;
        (value.is_finite()
            && self.null_min.is_none_or(|min| value > min)
            && self.null_max.is_none_or(|max| value < max))
        .then_some(value)
    }
}

fn bits(bytes: &[u8], start: usize, width: usize) -> Option<u32> {
    if width == 0 || width > 32 || start.checked_add(width)? > bytes.len().checked_mul(8)? {
        return None;
    }
    let mut value = 0;
    for bit in start..start + width {
        value = (value << 1) | u32::from((bytes[bit / 8] >> (7 - bit % 8)) & 1);
    }
    Some(value)
}

/// Offline numeric field definitions for all measurement-bearing PIDs in PIDS.MD.
/// Status/enumeration-only PIDs and unverified slots have no analog definitions.
pub fn fields() -> &'static [Field] {
    static FIELDS: OnceLock<Vec<Field>> = OnceLock::new();
    FIELDS.get_or_init(|| {
        include_str!("pids/fields.tsv")
            .lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
            .map(|line| {
                let c: Vec<_> = line.split('\t').collect();
                assert_eq!(c.len(), 15, "bundled PID definition column count");
                let number = |i: usize| c[i].parse::<f64>().expect("bundled PID number");
                let optional = |i: usize| (!c[i].is_empty()).then(|| number(i));
                let flags = |i: usize| {
                    c[i].split(',')
                        .filter(|s| !s.is_empty())
                        .map(|s| s.parse().expect("bundled PID flag offset"))
                        .collect()
                };
                Field {
                    pid: u8::from_str_radix(c[0], 16).expect("bundled PID ID"),
                    key: c[1],
                    name: c[2],
                    unit: c[3],
                    bit: c[4].parse().expect("bundled PID bit offset"),
                    width: c[5].parse().expect("bundled PID bit width"),
                    signed: c[6] == "1",
                    multiplier: number(7),
                    divisor: number(8),
                    offset: number(9),
                    null_min: optional(10),
                    null_max: optional(11),
                    support: flags(12),
                    unavailable: flags(13),
                    wide: (!c[14].is_empty())
                        .then(|| c[14].parse().expect("bundled wide-range flag")),
                }
            })
            .collect()
    })
}

/// Resolve a viewer-derived identity without interpreting arbitrary vendor signals.
pub fn field(signal: &str) -> Option<&'static Field> {
    let (pid, key) = signal.strip_prefix("01:")?.split_once(":decoded:")?;
    if pid.len() != 2 {
        return None;
    }
    let pid = u8::from_str_radix(pid, 16).ok()?;
    fields()
        .iter()
        .find(|field| field.pid == pid && field.key == key)
}
