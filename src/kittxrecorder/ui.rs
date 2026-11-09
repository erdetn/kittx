//! Reusable native UI for standalone applications and KittxLauncher.
//! Call `gpui_kit::init` once before opening windows. The host owns the event
//! loop and exclusivity between tools; opening a window does not connect an ECU.
use crate::app::{Quit, RecorderApp, Start, Stop};
use gpui_kit::*;

#[derive(Default)]
struct State {
    window: Option<(AnyWindowHandle, WeakEntity<RecorderApp>)>,
}
impl Global for State {}

/// Open this tool inside the host's GPUI event loop. Closing it leaves other
/// host windows alive. Recorder/Probe complete their normal finalization first.
pub fn open(cx: &mut App) -> Result<AnyWindowHandle, String> {
    if !cx.has_global::<State>() {
        cx.bind_keys([
            KeyBinding::new("secondary-r", Start, Some("Recorder")),
            KeyBinding::new("secondary-shift-r", Stop, Some("Recorder")),
            KeyBinding::new("secondary-q", Quit, Some("Recorder")),
        ]);

        cx.set_global(State::default());
        cx.on_action(|_: &Quit, cx| {
            if let Some((handle, _)) = cx.global::<State>().window.as_ref() {
                let handle = *handle;
                cx.defer(move |cx| {
                    let _ = close(handle, cx);
                });
            }
        });
    }

    cx.set_menus(vec![
        Menu::new("KittxRecorder").items([MenuItem::action("Quit", Quit)]),
        Menu::new("File").items([
            MenuItem::action("Start recording…", Start),
            MenuItem::action("Stop & save", Stop),
            MenuItem::separator(),
            MenuItem::action("Exit", Quit),
        ]),
    ]);
    if let Some((handle, _)) = cx.global::<State>().window.as_ref()
        && cx.windows().contains(handle)
    {
        return Err("This tool is already open".into());
    }
    let (handle, view) = gpui_kit::open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("KittxRecorder".into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1100.), px(900.)),
                cx,
            ))),

            ..Default::default()
        },
        cx,
        |window, cx| cx.new(|cx| RecorderApp::new(window, cx)),
    )
    .map_err(|error| error.to_string())?;
    cx.global_mut::<State>().window = Some((handle, view.downgrade()));
    Ok(handle)
}

/// Request the tool's normal close action, including stop/save handling.
/// The window may remain open while work is finalized or a save dialog is open.
pub fn close(handle: AnyWindowHandle, cx: &mut App) -> Result<(), String> {
    let (current, view) = cx
        .try_global::<State>()
        .and_then(|state| state.window.clone())
        .ok_or("This tool is not open")?;
    if current != handle {
        return Err("This window does not belong to this tool".into());
    }
    handle
        .update(cx, |_, window, cx| {
            view.update(cx, |app, cx| app.exit(window, cx))
        })
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

/// Run this project's standalone UI. Hosts such as KittxLauncher use `open`.
pub fn run() {
    application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            open(cx).expect("open KittxRecorder window");
            cx.activate(true);
        });
}
