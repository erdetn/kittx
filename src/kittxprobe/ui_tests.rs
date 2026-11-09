use super::{Field, ProbeApp};
use core::prelude::rust_2024::test;
use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, InputEvent, Point, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
#[path = "tests/common/mod.rs"]
mod common;
use crate::test_fixture as fixture;

fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<ProbeApp>) {
    cx.update(gpui_kit::init);
    cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1440.), px(960.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| ProbeApp::new(window, cx)),
        )
        .unwrap()
    })
}

fn wait_until(cx: &mut TestAppContext, mut ready: impl FnMut(&mut TestAppContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        if ready(cx) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "app did not reach expected state"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn toggle_pid(cx: &mut TestAppContext, handle: AnyWindowHandle, query: &str) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("measurements", cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
        window.input(query, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn connection_selection_and_scrollable_unit_tracks_do_not_overlap(cx: &mut TestAppContext) {
    let f = fixture::Fixture::new();
    for id in 1..=3 {
        f.sample(id, id as f64);
    }
    f.scan(1, "ok", Some(1));
    f.db.execute_batch(
        "INSERT INTO signal_definition VALUES('01:0C',1,12,'rpm','1',0);
        INSERT INTO signal_definition VALUES('adapter:voltage',NULL,NULL,'V','1',0);
        UPDATE measurement SET signal_id='01:0C',unit='rpm' WHERE id=2;
        UPDATE measurement SET signal_id='adapter:voltage',unit='V' WHERE id=3;",
    )
    .unwrap();
    let snapshot = crate::live::LiveSnapshot::read(&f.path, false, 400).unwrap();
    let (handle, app) = open(cx);
    cx.update(|cx| {
        app.update(cx, |app, cx| {
            app.live = Some(Arc::new(snapshot));
            cx.notify();
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let side = window.find("settings").bounds();
        let live = window.find("live-panel").bounds();
        assert!(side.right() <= live.left());
        let first = window.find(("unit-plot", 0usize)).bounds();
        let second = window.find(("unit-plot", 1usize)).bounds();
        assert!(first.size.height >= px(160.));
        assert!(second.top() >= first.bottom());
        let dtcs = window.find("probe-dtcs").bounds();
        // Wheel over the time label/gutter scrolls tracks; over a drawing it zooms.
        window.dispatch_event(
            gpui_kit::ScrollWheelEvent {
                position: gpui_kit::point(first.left() + px(10.), first.bottom() + px(5.)),
                delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-400.))),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert!(window.find(("unit-plot", 0usize)).bounds().top() < first.top());
        assert_eq!(window.find("probe-dtcs").bounds(), dtcs);
        window.click("theme", cx);
        assert!(!cx.theme().is_dark());
        window.click("theme", cx);
        assert!(cx.theme().is_dark());
        if cfg!(feature = "serial") {
            window.click("serial", cx);
            assert!(app.read(cx).serial);
            assert!(window.find("serial-port").bounds().size.height > px(0.));
        }
        window.click("tcp", cx);
        assert!(!app.read(cx).serial);
        window.click(("trigger", 0usize), cx);
        window.click(("trigger", 1usize), cx);
        window.scroll(
            "settings",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-600.))),
            cx,
        );
        window.render_frame(cx);
        window.click("clear-signals", cx);
    })
    .unwrap();
    toggle_pid(cx, handle, "Engine RPM");
    cx.update(|cx| assert_eq!(app.read(cx).settings(cx).unwrap().selected, [0x0c].into()));
    toggle_pid(cx, handle, "Engine RPM");
    cx.update(|cx| assert!(app.read(cx).settings(cx).unwrap().selected.is_empty()));
    cx.simulate_window_resize(handle, size(px(820.), px(900.)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let side = window.find("settings").bounds();
        let picker = window.find("measurements").bounds();
        assert!(picker.left() >= side.left() && picker.right() <= side.right());
        app.update(cx, |app, cx| {
            app.dtcs = [false; 3];
            cx.notify();
        });
        window.click("start", cx);
        assert!(app.read(cx).job.is_none());
        assert!(app.read(cx).message.contains("at least one"));
        window.click("exit", cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn plot_buttons_wheel_drag_and_reset_share_one_view_across_unit_groups(cx: &mut TestAppContext) {
    let f = fixture::Fixture::new();
    for second in 0..=10 {
        f.sample(second * 10_000 + 1, second as f64);
    }
    f.db.execute_batch(
        "INSERT INTO signal_definition VALUES('01:0C',1,12,'rpm','1',0);
        UPDATE measurement SET signal_id='01:0C',unit='rpm' WHERE id IN (20001,40001);
        UPDATE measurement SET ecu_id=2 WHERE id=10001;",
    )
    .unwrap();
    let snapshot = crate::live::LiveSnapshot::read(&f.path, false, 400).unwrap();
    assert_eq!(snapshot.tracks.len(), 2);
    assert_eq!(snapshot.tracks[0].unit, "km/h");
    assert_eq!(
        snapshot.tracks[0].signals.len(),
        2,
        "same unit, separate ECUs"
    );
    assert_eq!(snapshot.tracks[1].unit, "rpm");
    let full = snapshot.full;
    let (handle, app) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        // Controls have no effect before data arrives.
        window.click("plot-zoom-in", cx);
        assert!(app.read(cx).plot_view.is_none());
        app.update(cx, |app, cx| {
            app.live = Some(Arc::new(snapshot));
            cx.notify();
        });
        window.render_frame(cx);
        window.click("plot-zoom-in", cx);
        let zoomed = app.read(cx).plot_viewport().unwrap();
        assert_eq!(zoomed.to - zoomed.from, 50_000_000);
        window.click("plot-earlier", cx);
        let earlier = app.read(cx).plot_viewport().unwrap();
        assert!(earlier.from < zoomed.from);
        window.click("plot-later", cx);
        assert_eq!(app.read(cx).plot_viewport(), Some(zoomed));
        let before_scroll = window.find(("unit-plot", 0usize)).bounds();
        window.dispatch_event(
            gpui_kit::ScrollWheelEvent {
                position: gpui_kit::point(
                    before_scroll.left() + before_scroll.size.width / 4.,
                    before_scroll.center().y,
                ),
                delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(40.))),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        let wheel = app.read(cx).plot_viewport().unwrap();
        assert!(wheel.to - wheel.from < zoomed.to - zoomed.from);
        assert!(
            (wheel.time(0.25) - zoomed.time(0.25)).abs() <= 1,
            "pointer time stays anchored"
        );
        assert_eq!(
            window.find(("unit-plot", 0usize)).bounds(),
            before_scroll,
            "zoom does not scroll tracks"
        );
        let from = before_scroll.center();
        window.drag(
            from,
            gpui_kit::point(from.x + before_scroll.size.width / 8., from.y),
            cx,
        );
        let panned = app.read(cx).plot_viewport().unwrap();
        assert!(panned.from < wheel.from);
        assert_eq!(panned.to - panned.from, wheel.to - wheel.from);
        assert!(app.read(cx).plot_drag.is_none());
        window.scroll(
            ("unit-plot", 0usize),
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-40.))),
            cx,
        );
        let wheel_out = app.read(cx).plot_viewport().unwrap();
        assert!(wheel_out.to - wheel_out.from > panned.to - panned.from);
        window.click("plot-zoom-out", cx);
        let wider = app.read(cx).plot_viewport().unwrap();
        assert!(wider.to - wider.from > wheel_out.to - wheel_out.from);
        window.click("plot-reset", cx);
        assert!(app.read(cx).plot_view.is_none());
        assert_eq!(app.read(cx).plot_viewport(), Some(full));
        window.click("theme", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.simulate_window_resize(handle, size(px(820.), px(900.)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let plot = window.find(("unit-plot", 0usize)).bounds();
        assert!(plot.size.height >= px(160.));
        assert!(plot.right() <= px(820.));
        window.click("exit", cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn live_start_stop_selected_dtcs_save_cancel_failure_and_retry(cx: &mut TestAppContext) {
    let server = common::ScriptServer::start();
    let (handle, app) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.fields[&Field::Port].update(cx, |input, cx| {
                input.set_value(server.port.to_string(), window, cx)
            });
            app.fields[&Field::SamplePeriod]
                .update(cx, |input, cx| input.set_value("10", window, cx));
            app.fields[&Field::Timeout].update(cx, |input, cx| input.set_value("1", window, cx));
            app.select_measurements(vec![0x0c, 0x61], window, cx);
            app.dtcs = [true, false, false];
        });
        window.render_frame(cx);
        window.click("start", cx);
        assert!(app.read(cx).active());
        assert!(
            !app.read(cx).choosing,
            "Start must not prompt for a save path"
        );
        window.click("save", cx);
        assert!(!app.read(cx).choosing);
        window.click("serial", cx);
        assert!(
            !app.read(cx).serial,
            "transport settings lock during capture"
        );
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| {
            app.read(cx)
                .live
                .as_ref()
                .is_some_and(|s| s.tracks.len() == 2 && !s.dtcs.is_empty())
        })
    });
    let path = cx.update(|cx| app.read(cx).status.as_ref().unwrap().path.clone());
    let manual_view = cx
        .update_window(handle, |_, window, cx| {
            // Several requests while a read is in flight must leave the latest view.
            window.click("plot-zoom-in", cx);
            window.click("plot-zoom-out", cx);
            window.click("plot-zoom-in", cx);
            app.read(cx).plot_view.unwrap()
        })
        .unwrap();
    let previous_cutoff = cx.update(|cx| app.read(cx).live.as_ref().unwrap().cutoff);
    wait_until(cx, |cx| {
        cx.update(|cx| {
            let app = app.read(cx);
            app.live
                .as_ref()
                .is_some_and(|s| s.view == manual_view && s.cutoff > previous_cutoff)
        })
    });
    cx.update_window(handle, |_, window, cx| window.click("plot-reset", cx))
        .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| {
            let app = app.read(cx);
            app.plot_view.is_none() && !app.live_busy && app.live_message.starts_with("Live")
        })
    });
    let manual_view = cx
        .update_window(handle, |_, window, cx| {
            window.click("plot-zoom-in", cx);
            app.read(cx).plot_view.unwrap()
        })
        .unwrap();
    cx.update(|cx| {
        let app = app.read(cx);
        assert!(app.active());
        assert!(
            app.live
                .as_ref()
                .unwrap()
                .dtcs
                .iter()
                .all(|d| d.scan.category == "stored")
        );
    });
    cx.update_window(handle, |_, window, cx| window.click("stop", cx))
        .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| {
            let app = app.read(cx);
            !app.active() && !app.live_busy && app.live_message.starts_with("Inspecting")
        })
    });
    cx.update(|cx| assert_eq!(app.read(cx).plot_viewport(), Some(manual_view)));
    cx.update_window(handle, |_, window, cx| window.click("plot-reset", cx))
        .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| {
            let app = app.read(cx);
            !app.live_busy && app.live_message.starts_with("Stopped")
        })
    });
    cx.update(|cx| {
        let app = app.read(cx);
        assert!(app.plot_view.is_none());
        let snapshot = app.live.as_ref().unwrap();
        assert_eq!(snapshot.view, snapshot.full);
    });
    let requests = server.finish();
    assert!(requests.iter().any(|r| r == "010C"));
    assert!(requests.iter().any(|r| r == "0161"));
    for request in ["010D", "07", "0A", "04"] {
        assert!(!requests.iter().any(|r| r == request));
    }
    cx.update_window(handle, |_, window, cx| {
        assert!(
            app.read(cx).can_start(),
            "saving is optional before another Start"
        );
        window.click("save", cx);
        assert!(app.read(cx).choosing);
        assert!(!app.update(cx, |app, cx| app.request_close(window, cx)));
    })
    .unwrap();
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!app.read(cx).choosing);
        assert!(app.read(cx).message.contains("cancelled"));
        assert!(app.read(cx).job.as_ref().unwrap().saved_to().is_none());
    });
    let existing = path.parent().unwrap().join("existing.kittx");
    std::fs::write(&existing, b"keep this file").unwrap();
    cx.update_window(handle, |_, window, cx| window.click("save", cx))
        .unwrap();
    cx.simulate_new_path_selection(|_| Some(existing.clone()));
    wait_until(cx, |cx| cx.update(|cx| !app.read(cx).choosing));
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep this file");
    cx.update(|cx| assert!(app.read(cx).message.contains("Save failed")));
    let saved = path.parent().unwrap().join("saved.kittx");
    cx.update_window(handle, |_, window, cx| window.click("save", cx))
        .unwrap();
    cx.simulate_new_path_selection(|_| Some(saved.clone()));
    wait_until(cx, |cx| cx.update(|cx| !app.read(cx).choosing));
    cx.update(|cx| {
        assert_eq!(
            app.read(cx).job.as_ref().unwrap().saved_to(),
            Some(saved.as_path())
        );
        assert!(app.read(cx).can_start());
    });
    let doc = kittx_viewer::document::Document::open(saved).unwrap();
    assert_eq!(doc.overview.session.status, "completed");
    assert!(doc.overview.producer.starts_with("KittxProbe"));
    cx.update_window(handle, |_, window, cx| window.click("exit", cx))
        .unwrap();
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[gpui_kit::test]
fn close_during_capture_waits_for_finalization_and_retains_working_file(cx: &mut TestAppContext) {
    active_capture_close(cx, false);
}

#[gpui_kit::test]
fn hosted_close_finalizes_capture_and_preserves_host(cx: &mut TestAppContext) {
    active_capture_close(cx, true);
}

struct TestHost;
impl gpui_kit::Render for TestHost {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
    }
}

fn active_capture_close(cx: &mut TestAppContext, hosted: bool) {
    let server = common::ScriptServer::start();
    let (handle, app) = open(cx);
    let host = hosted.then(|| {
        cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| TestHost))
                .unwrap()
                .0
        })
    });
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.fields[&Field::Port].update(cx, |input, cx| {
                input.set_value(server.port.to_string(), window, cx)
            });
            app.fields[&Field::Timeout].update(cx, |input, cx| input.set_value("1", window, cx));
            app.select_measurements(vec![0x0c], window, cx);
            app.dtcs = [false; 3];
        });
        window.click("start", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| {
            app.read(cx)
                .status
                .as_ref()
                .is_some_and(|s| s.phase == kittx_recorder::recording::Phase::Recording)
        })
    });
    let path = cx.update(|cx| app.read(cx).status.as_ref().unwrap().path.clone());
    cx.update_window(handle, |_, window, cx| {
        window.click("exit", cx);
        assert!(app.read(cx).closing);
        assert!(app.read(cx).close_after_stop);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| cx.windows() == host.into_iter().collect::<Vec<_>>())
    });
    server.finish();
    assert_eq!(
        kittx_viewer::document::Document::open(path.clone())
            .unwrap()
            .overview
            .session
            .status,
        "completed"
    );
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[gpui_kit::test]
fn restarting_without_saving_retains_the_previous_working_capture(cx: &mut TestAppContext) {
    let (handle, app) = open(cx);
    let mut paths = Vec::new();
    for _ in 0..2 {
        let server = common::ScriptServer::start();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.fields[&Field::Port].update(cx, |input, cx| {
                    input.set_value(server.port.to_string(), window, cx)
                });
                app.fields[&Field::Timeout]
                    .update(cx, |input, cx| input.set_value("1", window, cx));
                app.select_measurements(vec![0x0c], window, cx);
                app.dtcs = [false; 3];
            });
            window.click("start", cx);
        })
        .unwrap();
        wait_until(cx, |cx| {
            cx.update(|cx| {
                app.read(cx)
                    .status
                    .as_ref()
                    .is_some_and(|s| s.phase == kittx_recorder::recording::Phase::Recording)
            })
        });
        let path = cx.update(|cx| app.read(cx).status.as_ref().unwrap().path.clone());
        cx.update_window(handle, |_, window, cx| window.click("stop", cx))
            .unwrap();
        wait_until(cx, |cx| {
            cx.update(|cx| !app.read(cx).active() && !app.read(cx).live_busy)
        });
        server.finish();
        assert_eq!(
            kittx_viewer::document::Document::open(path.clone())
                .unwrap()
                .overview
                .session
                .status,
            "completed"
        );
        paths.push(path);
    }
    cx.update(|cx| assert_eq!(app.read(cx).previous_unsaved.as_ref(), Some(&paths[0])));
    assert_ne!(paths[0], paths[1]);
    assert!(paths.iter().all(|path| path.is_file()));
    cx.update_window(handle, |_, window, cx| window.click("exit", cx))
        .unwrap();
    for path in paths {
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
