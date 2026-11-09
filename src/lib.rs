//! Synchronous ELM327 communication over raw TCP and desktop serial ports.
//!
//! Operations block and invoke their callback once on the calling thread. A
//! mutable client owns one transaction at a time; applications own scheduling.
//! See the repository README for initialization, timeouts and monitoring.
//!
//! ```no_run
//! use kittx::{Client, TransportConfig, Initialization, Protocol};
//! let mut client = Client::with_initialization(
//!     TransportConfig::tcp("127.0.0.1", 35000),
//!     Initialization::obd(Protocol::Automatic),
//! );
//! client.connect(None, |result| println!("{result:?}"));
//! client.read_voltage(None, |result| println!("{result:?}"));
//! client.disconnect(None, |result| println!("{result:?}"));
//! ```

#![forbid(unsafe_code)]

mod client;
pub mod command;
#[cfg(feature = "datastore")]
#[doc = include_str!("../README_DATASTORED.md")]
pub mod datastore;
pub mod error;
pub mod framing;
pub mod obd;
pub mod response;
pub mod transport;

pub use client::{
    Client, ClientOptions, Initialization, MonitorControl, MonitorSummary, ObdEvidence,
    SessionState, Settings,
};
pub use command::{Command, Protocol};
pub use error::{Error, ErrorKind, Recovery, Result};
pub use response::{Firmware, Response, ResponseValue, Transaction};
pub use transport::{SerialConfig, TransportConfig};
