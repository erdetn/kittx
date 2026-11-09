//! Application models and synchronous recording worker for KittxRecorder.
mod diagnostics;
pub mod model;
mod pid_catalog;
pub mod recording;

#[cfg(feature = "ui")]
mod app;
#[cfg(feature = "ui")]
pub mod ui;
#[cfg(feature = "ui")]
#[path = "../ui_theme.rs"]
mod ui_theme;

#[cfg(feature = "ui")]
mod serial_ports;
#[cfg(feature = "ui")]
mod vehicle;
