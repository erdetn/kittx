use super::*;
use core::prelude::rust_2024::test;
use gpui_kit::test::TestWindowExt;

fn launcher(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Launcher>) {
    cx.update(gpui_kit::init);
    cx.update(|cx| {
        install_menu(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Launcher::new(window, cx))
        })
        .unwrap()
    })
}

#[gpui_kit::test]
fn all_four_real_tool_windows_open_in_host_and_return_after_exit(cx: &mut TestAppContext) {
    let (parent, app) = launcher(cx);
    // Repeat to catch stale Quit handlers and window-close subscriptions.
    for _ in 0..2 {
        for tool in Tool::ALL {
            cx.update_window(parent, |_, window, cx| {
                window.render_frame(cx);
                window.click(tool.label(), cx);
            })
            .unwrap();
            let handle = app.update(cx, |app, cx| {
                let (chosen, handle) = app.running.unwrap();
                assert_eq!(chosen, tool);
                assert_eq!(cx.windows().len(), 2);
                assert!(tool.open(cx).is_err());
                app.open(Tool::Probe, cx);
                assert_eq!(app.running.unwrap().1, handle);
                assert_eq!(cx.windows().len(), 2);
                handle
            });
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click("exit", cx);
            })
            .unwrap();
            cx.run_until_parked();
            app.update(cx, |app, cx| {
                assert!(app.running.is_none());
                assert_eq!(cx.windows(), vec![parent]);
                assert_eq!(app.status, "Choose an application to open.");
            });
        }
    }
}

#[gpui_kit::test]
fn tool_close_api_keyboard_quit_and_launcher_close_keep_host_alive(cx: &mut TestAppContext) {
    let (parent, app) = launcher(cx);
    for tool in Tool::ALL {
        let handle = app.update(cx, |app, cx| {
            app.open(tool, cx);
            app.running.unwrap().1
        });
        // The public close API rejects another host window.
        cx.update(|cx| assert!(tool.close(parent, cx).is_err()));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press(
                if cfg!(target_os = "macos") {
                    "cmd-q"
                } else {
                    "ctrl-q"
                },
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            assert!(app.running.is_none());
            assert_eq!(cx.windows(), vec![parent]);
        });
        app.update(cx, |app, cx| app.open(tool, cx));
        cx.update_window(parent, |_, window, cx| {
            app.update(cx, |app, cx| app.exit(window, cx));
        })
        .unwrap();
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            assert!(app.running.is_none());
            assert_eq!(cx.windows(), vec![parent]);
        });
    }
}
