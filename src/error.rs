//! Errors retain partial wire evidence and synchronization outcome.
use std::{fmt, io};

/// Library operation result.
pub type Result<T> = std::result::Result<T, Error>;

/// Recovery after a failed operation; never implies an automatic retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    NotNeeded,
    Synchronized,
    ReconnectRequired,
}

/// Stable error categories, independent of adapter text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidArgument,
    Disconnected,
    AlreadyConnected,
    UnusableSession,
    UnsupportedTransport,
    UnsupportedFirmware,
    UnknownCapability,
    Io(io::ErrorKind),
    Timeout,
    Rejected,
    BridgeFault,
    NoData,
    MalformedResponse,
    EcuNegative { service: u8, code: u8 },
    AmbiguousSender,
    IncompleteMessage,
    BufferOverflow,
    Reset,
    Initialization { step: usize },
}

/// An operation failure. `partial` may contain useful data even on failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    pub partial: Vec<u8>,
    pub recovery: Recovery,
    pub cause: Option<Box<Error>>,
}
impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            partial: Vec::new(),
            recovery: Recovery::NotNeeded,
            cause: None,
        }
    }
    pub(crate) fn io(error: io::Error) -> Self {
        let kind = if matches!(
            error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ) {
            ErrorKind::Timeout
        } else {
            ErrorKind::Io(error.kind())
        };
        Self::new(kind, error.to_string())
    }
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidArgument, message)
    }
    pub(crate) fn malformed(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::MalformedResponse, message)
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_deref().map(|e| e as _)
    }
}
