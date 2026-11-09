//! Synchronous ECU recordings in versioned SQLite `.kittx` documents.
//! Enable the `datastore` feature. See `README_DATASTORED.md` for file semantics.
mod model;
mod reader;
mod recorder;
mod source;
mod store;
mod view;

pub use model::*;
pub use reader::*;
pub use recorder::*;
pub use source::*;
pub use view::*;

/// Recording/storage failures are separate from communication errors retained in observations.
#[derive(Debug)]
pub enum DataError {
    Invalid(String),
    State(String),
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    Source(crate::Error),
}
impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(s) | Self::State(s) => f.write_str(s),
            Self::Io(e) => e.fmt(f),
            Self::Sqlite(e) => e.fmt(f),
            Self::Source(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for DataError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Sqlite(e) => Some(e),
            Self::Source(e) => Some(e),
            _ => None,
        }
    }
}
impl From<std::io::Error> for DataError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<rusqlite::Error> for DataError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}
impl From<crate::Error> for DataError {
    fn from(e: crate::Error) -> Self {
        Self::Source(e)
    }
}
pub type DataResult<T> = std::result::Result<T, DataError>;
pub(crate) fn invalid(s: impl Into<String>) -> DataError {
    DataError::Invalid(s.into())
}
