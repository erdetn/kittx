// Preserve the project's standalone executable alongside the reusable UI.
#![allow(non_snake_case)]
#[path = "../ui_session.rs"]
mod ui_session;

fn main() {
    let _session = ui_session::UiSession::acquire("desktop-ui").unwrap_or_else(|error| {
        eprintln!("Cannot start Kittx UI: {error}");
        std::process::exit(1);
    });
    kittx_probe::ui::run();
}
