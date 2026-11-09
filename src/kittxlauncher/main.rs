#![allow(non_snake_case)]
mod app;
mod launch;
#[path = "../ui_session.rs"]
mod ui_session;
#[path = "../ui_theme.rs"]
mod ui_theme;

use app::{Launcher, Quit};
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

fn main() {
    let _session = ui_session::UiSession::acquire("desktop-ui").unwrap_or_else(|error| {
        eprintln!("Cannot start KittxLauncher: {error}");
        std::process::exit(1);
    });
    application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            app::install_menu(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let (handle, view) = gpui_kit::open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("KittxLauncher".into()),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(560.), px(480.)),
                        cx,
                    ))),
                    window_min_size: Some(size(px(440.), px(440.))),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| Launcher::new(window, cx)),
            )
            .expect("open KittxLauncher window");
            let weak = view.downgrade();
            cx.on_action(move |_: &Quit, cx| {
                let weak = weak.clone();
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        let _ = weak.update(cx, |app, cx| app.exit(window, cx));
                    });
                });
            });
            cx.activate(true);
        });
}
