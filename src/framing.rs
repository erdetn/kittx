//! Incremental serial framing independent of transports and ECU schemas.
use crate::{Error, ErrorKind, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameEvent {
    Line(String),
    Prompt,
}

/// Buffers only one line. Transaction capture limits are enforced by the client.
#[derive(Debug, Clone)]
pub struct Decoder {
    cr: u8,
    lf: u8,
    limit: usize,
    line: Vec<u8>,
}
impl Decoder {
    pub fn new(cr: u8, lf: u8, limit: usize) -> Result<Self> {
        if cr == 0 || cr == b'>' || lf == b'>' || cr == lf || limit == 0 {
            return Err(Error::invalid("invalid framing configuration"));
        }
        Ok(Self {
            cr,
            lf,
            limit,
            line: Vec::new(),
        })
    }
    /// Feed a byte; NULL bytes are ignored. Prompt is recognized at a line
    /// boundary, allowing `>` inside printable identifier/description strings.
    pub fn push(&mut self, byte: u8) -> Result<Option<FrameEvent>> {
        self.push_with_prompt(byte, true)
    }
    pub(crate) fn push_literal(&mut self, byte: u8) -> Result<Option<FrameEvent>> {
        self.push_with_prompt(byte, false)
    }
    fn push_with_prompt(&mut self, byte: u8, prompt: bool) -> Result<Option<FrameEvent>> {
        if byte == 0 {
            return Ok(None);
        }
        if prompt && byte == b'>' && self.line.is_empty() {
            return Ok(Some(FrameEvent::Prompt));
        }
        if byte == self.cr || byte == self.lf {
            if self.line.is_empty() {
                return Ok(None);
            }
            let bytes = std::mem::take(&mut self.line);
            let s =
                String::from_utf8(bytes).map_err(|_| Error::malformed("non-UTF8 serial line"))?;
            return Ok(Some(FrameEvent::Line(s.trim().to_owned())));
        }
        if self.line.len() >= self.limit {
            return Err(Error::new(
                ErrorKind::BufferOverflow,
                "input line limit exceeded",
            ));
        }
        self.line.push(byte);
        Ok(None)
    }
    /// Unfinished bytes, used for LP's deliberately unterminated OK.
    pub fn pending(&self) -> &[u8] {
        &self.line
    }
    pub fn clear(&mut self) {
        self.line.clear();
    }
}
