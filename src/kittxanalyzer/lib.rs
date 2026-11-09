//! Read-only recording evidence and AI service integration for KittxAnalyzer.
//! Network requests belong to this application package, not the ELM327 library.
pub mod ai;
pub mod model;
mod prompt;

#[cfg(feature = "ui")]
mod app;
#[cfg(feature = "ui")]
pub mod ui;
#[cfg(feature = "ui")]
#[path = "../ui_theme.rs"]
mod ui_theme;
