//! Read-only viewer models and optional reusable GPUI UI, also available as
//! the standalone `kittx-viewer` binary.
pub mod document;
pub mod model;
pub mod pid;
mod pid_view;

#[cfg(feature = "ui")]
mod app;
#[cfg(feature = "ui")]
pub mod ui;
#[cfg(feature = "ui")]
#[path = "../ui_theme.rs"]
mod ui_theme;
