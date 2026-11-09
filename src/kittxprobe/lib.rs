//! Application-owned live capture, read-only plot projections and save-after-stop.
//! ECU requests run through KittxRecorder's synchronous worker; the UI never
//! accesses the adapter or modifies the recording through its plot reader.
pub mod capture;
pub mod live;

#[cfg(test)]
#[path = "../../tests/common/viewer_fixture.rs"]
mod test_fixture;

#[cfg(feature = "ui")]
mod app;
#[cfg(feature = "ui")]
pub mod ui;
#[cfg(feature = "ui")]
#[path = "../ui_theme.rs"]
mod ui_theme;

#[cfg(feature = "ui")]
mod plot_ui;
#[cfg(feature = "ui")]
#[path = "../kittxrecorder/serial_ports.rs"]
mod serial_ports;
#[cfg(feature = "ui")]
#[path = "../kittxrecorder/vehicle.rs"]
mod vehicle;
