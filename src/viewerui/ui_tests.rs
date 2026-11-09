use super::{Document, Viewer};
use core::prelude::rust_2024::test;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size};
#[path = "../../tests/common/viewer_fixture.rs"]
mod fixture;

fn toggle_pid(cx: &mut TestAppContext, handle: gpui_kit::AnyWindowHandle, query: &str) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("pid-picker", cx);
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
}

#[gpui_kit::test]
fn pid_combobox_toggles_by_identity_and_shares_bulk_and_legend_selection(cx: &mut TestAppContext) {
    let f = fixture::Fixture::new();
    f.sample(1, 0.);
    f.sample(2, 0.);
    f.db.execute_batch("INSERT INTO signal_definition VALUES('01:42:raw',1,66,'raw','1',1);
        UPDATE measurement SET signal_id='01:42:raw',unit='raw',value_type='blob',number_value=NULL,blob_value=X'41423070';
        UPDATE measurement SET ecu_id=2 WHERE id=2;").unwrap();
    let doc = Document::open(f.path.clone()).unwrap();
    let first = doc
        .overview
        .series
        .iter()
        .position(|s| s.numeric && s.ecu_id == 1)
        .unwrap();
    let second = doc
        .overview
        .series
        .iter()
        .position(|s| s.numeric && s.ecu_id == 2)
        .unwrap();
    cx.update(gpui_kit::init);
    let (handle, view) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1440.), px(900.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let mut viewer = Viewer::new(window, cx);
                    viewer.view = doc.full();
                    viewer.document = Some(doc);
                    viewer
                })
            },
        )
        .unwrap()
    });
    let combo_id = cx.update(|cx| ("multi-combo-box", view.read(cx).signals.entity_id()));
    // Numeric entries sort before raw sources. Selection must use the original
    // series identity, not the dropdown's sorted or searched row number.
    toggle_pid(cx, handle, "V · ECU 7E8");
    cx.update(|cx| {
        let viewer = view.read(cx);
        assert_eq!(viewer.selected, [first].into());
        let frame = viewer.frame.as_ref().unwrap();
        assert_eq!(frame.samples.len(), 1);
        assert_eq!(frame.samples[0].0, first);
        assert_eq!(frame.samples[0].1[0].value, 12.4);
        assert_eq!(
            viewer.document.as_ref().unwrap().overview.series[first].unit,
            "V"
        );
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(combo_id).expanded(), Some(true));
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    toggle_pid(cx, handle, "V · ECU 7E9");
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).selected, [first, second].into());
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    toggle_pid(cx, handle, "V · ECU 7E8");
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).selected, [second].into());
        window.press("escape", cx);
        window.click(("legend", second), cx);
        assert!(view.read(cx).selected.is_empty());
        assert!(view.read(cx).signals.read(cx).selected_values().is_empty());
    })
    .unwrap();
    cx.run_until_parked();
    // Status/raw-only rows are present but cannot become numeric selections.
    toggle_pid(cx, handle, "01:42:raw");
    cx.update_window(handle, |_, window, cx| {
        assert!(view.read(cx).selected.is_empty());
        assert!(view.read(cx).signals.read(cx).selected_values().is_empty());
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("ecu", 1usize), cx);
        window.click("all", cx);
        let viewer = view.read(cx);
        assert_eq!(viewer.selected, [first, second].into());
        assert_eq!(
            viewer.signals.read(cx).selected_values(),
            vec![first, second]
        );
        assert_eq!(viewer.active_selected(), vec![first]);
        window.click("every-ecu", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(view.read(cx).frame.as_ref().unwrap().samples.len(), 2));
    // An unmatched search and closing the dropdown keep all checked values.
    toggle_pid(cx, handle, "not a PID");
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).selected, [first, second].into());
        window.press("escape", cx);
        window.click("clear", cx);
        assert!(view.read(cx).selected.is_empty());
        assert!(view.read(cx).signals.read(cx).selected_values().is_empty());
        assert!(view.read(cx).dtcs);
        window.click("all", cx);
        assert_eq!(view.read(cx).selected, [first, second].into());
    })
    .unwrap();
    cx.run_until_parked();

    // Opening a different recording replaces the catalog and its checked values.
    let replacement = fixture::Fixture::new();
    replacement.sample(1, 80.);
    cx.update(|cx| view.update(cx, |v, cx| v.load(replacement.path.clone(), cx)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let viewer = view.read(cx);
        assert_eq!(viewer.selected, [0].into());
        assert_eq!(viewer.signals.read(cx).selected_values(), vec![0]);
        assert_eq!(
            viewer.document.as_ref().unwrap().overview.series[0].signal,
            "speed"
        );
    })
    .unwrap();
    toggle_pid(cx, handle, "km/h · ECU 7E8");
    cx.update_window(handle, |_, window, cx| {
        assert!(view.read(cx).selected.is_empty());
        window.press("escape", cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn dtc_points_highlight_pin_and_show_exact_codes_and_times(cx: &mut TestAppContext) {
    use gpui_kit::{InputEvent as _, MouseMoveEvent, point};
    let f = fixture::Fixture::new();
    f.db.execute("UPDATE session SET started_utc_us=created_utc_us", [])
        .unwrap();
    for id in 1..=100 {
        f.sample(id, id as f64);
    }
    f.scan(10, "ok", Some(1));
    f.scan(11, "ok", Some(1));
    f.scan(90, "ok", Some(1));
    f.db.execute_batch("UPDATE dtc_scan_result SET category='pending',ecu_id=2 WHERE id=11;UPDATE acquisition SET operation_started_elapsed_us=10000,operation_completed_elapsed_us=10000 WHERE id=11;UPDATE dtc_entry SET code='P0456',code_bytes=X'0456' WHERE id=11;UPDATE dtc_entry SET code='P0300',code_bytes=X'0300' WHERE id=90;").unwrap();
    let doc = Document::open(f.path.clone()).unwrap();
    cx.update(gpui_kit::init);
    let (handle, view) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1440.), px(900.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let mut viewer = Viewer::new(window, cx);
                    viewer.focus.focus(window, cx);
                    viewer.view = doc.full();
                    viewer.selected.insert(0);
                    viewer.frame = Some(
                        doc.frame(doc.full(), &[0], 800, Default::default())
                            .unwrap(),
                    );
                    viewer.document = Some(doc);
                    viewer.details_visible = false;
                    viewer
                })
            },
        )
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let viewer = view.read(cx);
        let marker = viewer
            .dtc_points
            .iter()
            .find(|point| point.members.iter().any(|m| m.scan.id == 10))
            .unwrap();
        assert_eq!(marker.members.len(), 2);
        let position = point(
            viewer.bounds.left() + px(marker.x),
            viewer.bounds.bottom() - px(12.),
        );
        window.dispatch_event(
            MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert_eq!(view.read(cx).hovered_dtc, Some(10));
        assert!(!view.read(cx).pinned);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let viewer = view.read(cx);
        assert!(viewer.details.contains("Codes: P0123"));
        assert!(viewer.details.contains("Codes: P0456"));
        assert!(
            viewer
                .details
                .contains("Observed: 0.010000 s from recording start")
        );
        assert!(viewer.details.contains("UTC: 10000 µs"));
    });
    cx.update_window(handle, |_, window, cx| {
        let viewer = view.read(cx);
        let marker = viewer
            .dtc_points
            .iter()
            .find(|point| point.id == 10)
            .unwrap();
        let offset = point(px(marker.x), viewer.bounds.size.height - px(12.));
        let original = viewer.view;
        window.click_at("plot", offset, cx);
        let viewer = view.read(cx);
        assert!(viewer.pinned && viewer.details_visible);
        assert_eq!(viewer.selected_dtc, Some(10));
        assert!(viewer.drag.is_none());
        assert_eq!(viewer.view, original);
        window.render_frame(cx);
        let details = window.find("details");
        assert!(details.visible());
        assert!(details.bounds().size.height > px(0.));
        assert!(details.bounds().top() >= window.find("plot").bounds().bottom());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.scroll(
            "details",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-120.))),
            cx,
        );
        assert!(view.read(cx).details_scroll.offset().y < px(0.));
        let viewer = view.read(cx);
        let marker = viewer
            .dtc_points
            .iter()
            .find(|point| point.id == 90)
            .unwrap();
        let position = point(
            viewer.bounds.left() + px(marker.x),
            viewer.bounds.bottom() - px(12.),
        );
        let old_details = viewer.details.clone();
        window.dispatch_event(
            MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert_eq!(view.read(cx).hovered_dtc, Some(90));
        assert_eq!(view.read(cx).selected_dtc, Some(10));
        assert_eq!(view.read(cx).details, old_details);
        let offset = point(
            position.x - view.read(cx).bounds.left(),
            position.y - view.read(cx).bounds.top(),
        );
        window.click_at("plot", offset, cx);
        assert_eq!(view.read(cx).selected_dtc, Some(90));
        assert_eq!(view.read(cx).details_scroll.offset().y, px(0.));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let viewer = view.read(cx);
        assert!(viewer.details.contains("Codes: P0300"));
        assert!(
            viewer
                .details
                .contains("Observed: 0.090000 s from recording start")
        );
        assert!(!viewer.details.contains("P0123"));
    });
    cx.update_window(handle, |_, window, cx| {
        use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
        Theme::change(ThemeMode::Dark, Some(window), cx);
        window.render_frame(cx);
        window.click("theme", cx);
        assert!(!cx.theme().is_dark());
        assert_eq!(view.read(cx).selected_dtc, Some(90));
        window.click("theme", cx);
        assert!(cx.theme().is_dark());
        window.press("escape", cx);
        assert!(!view.read(cx).pinned);
        assert!(view.read(cx).selected_dtc.is_none());
        assert!(view.read(cx).hovered_dtc.is_none());
        let viewer = view.read(cx);
        let offset = point(
            viewer.bounds.size.width * 0.5,
            viewer.bounds.size.height - px(12.),
        );
        window.click_at("plot", offset, cx);
        assert!(view.read(cx).selected_dtc.is_none());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let details = &view.read(cx).details;
        assert!(details.contains("sample"));
        assert!(!details.contains("DTC read"));
    });
    // History rows select their exact read even when another point is pinned.
    cx.update_window(handle, |_, window, cx| {
        window.click(("history", 1usize), cx);
        assert_eq!(view.read(cx).selected_dtc, Some(11));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let details = &view.read(cx).details;
        assert!(details.contains("Codes: P0456"));
        assert!(!details.contains("Codes: P0123"));
    });
}

#[gpui_kit::test]
fn native_layout_buttons_and_keyboard(cx: &mut TestAppContext) {
    let f = fixture::Fixture::new();
    for i in 1..=10 {
        f.sample(i, i as f64)
    }
    f.scan(2, "ok", Some(1));
    let mut doc = Document::open(f.path.clone()).unwrap();
    doc.overview.producer = "Long descriptive producer metadata with provenance ".repeat(15);
    cx.update(gpui_kit::init);
    let (handle, view) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1440.), px(900.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let mut v = Viewer::new(window, cx);
                    v.focus.focus(window, cx);
                    v.view = doc.full();
                    v.selected.insert(0);
                    v.frame = Some(
                        doc.frame(doc.full(), &[0], 800, Default::default())
                            .unwrap(),
                    );
                    v.document = Some(doc);
                    v
                })
            },
        )
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.draw(cx).clear(cx);
        let info = window.find("info").bounds();
        let controls = window.find("controls").bounds();
        let chart = window.find("chart").bounds();
        assert!(info.right() <= controls.left());
        assert!(controls.right() <= chart.left());
        let producer = window.find(("metadata", 8usize)).bounds();
        assert!(producer.size.height > px(32.));
        assert!(producer.left() >= info.left() && producer.right() <= info.right());
        window.click(("trigger", 0usize), cx);
        assert!(!view.read(cx).metadata_sections.contains(&0));
        assert!(view.read(cx).metadata_sections.contains(&1));
        window.click(("trigger", 0usize), cx);
        assert!(view.read(cx).metadata_sections.contains(&0));
        let original = view.read(cx).view;
        window.click("plus", cx);
        assert!(view.read(cx).view.to - view.read(cx).view.from < original.to - original.from);
        window.click("mode", cx);
        assert!(view.read(cx).box_zoom);
        window.click("side", cx);
        assert!(!view.read(cx).sidebar);
        window.click("clear", cx);
        assert!(view.read(cx).selected.is_empty());
        window.click("all", cx);
        assert_eq!(view.read(cx).selected.len(), 1);
        window.click("dtc-mode", cx);
        assert_eq!(
            view.read(cx).dtc_mode,
            kittx::datastore::DtcDisplayMode::FirstObserved
        );
        view.update(cx, |v, _| {
            v.pinned = true;
            v.drag = Some((Point::default(), v.view));
        });
        window.press("escape", cx);
        assert!(!view.read(cx).pinned);
        assert!(view.read(cx).drag.is_none());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.click("open", cx))
        .unwrap();
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            view.read(cx).document.as_ref().unwrap().overview.session.id,
            "test"
        )
    });
    cx.update(|cx| view.update(cx, |v, cx| v.load(f.path.with_extension("missing"), cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        let v = view.read(cx);
        assert_eq!(v.document.as_ref().unwrap().overview.session.id, "test");
        assert!(v.status.contains("Cannot open"));
    });
    cx.update_window(handle, |_, window, cx| {
        use gpui_kit::component::ActiveTheme;
        gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, Some(window), cx);
        window.draw(cx).clear(cx);
        window.click("theme", cx);
        assert!(!cx.theme().is_dark());
        window.click("theme", cx);
        assert!(cx.theme().is_dark());
        assert!(view.read(cx).metadata_sections.contains(&0));
        assert!(view.read(cx).metadata_sections.contains(&1));
        window.draw(cx).clear(cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| window.click("side", cx))
        .unwrap();
    cx.update(|cx| {
        view.update(cx, |v, cx| {
            v.document.as_mut().unwrap().overview.series[0].sender =
                "Long ECU identity with explanatory text ".repeat(8);
            v.signal_catalog_dirty = true;
            cx.notify();
        })
    });
    cx.simulate_window_resize(handle, size(px(820.), px(900.)));
    cx.update_window(handle, |_, window, cx| {
        window.draw(cx).clear(cx);
        let info = window.find("info").bounds();
        let controls = window.find("controls").bounds();
        let chart = window.find("chart").bounds();
        assert!(info.right() <= controls.left());
        assert!(controls.right() <= chart.left());
        assert!(chart.right() <= px(821.));
        let picker = window.find("pid-picker").bounds();
        assert!(picker.left() >= controls.left() && picker.right() <= controls.right());
        for id in ["all", "clear"] {
            let button = window.find(id).bounds();
            assert!(button.top() >= picker.bottom());
            assert!(button.left() >= controls.left() && button.right() <= controls.right());
        }
        let producer = window.find(("metadata", 8usize)).bounds();
        assert!(producer.right() <= info.right());
        assert!(producer.size.height > px(32.));
        window.scroll(
            "info",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-300.))),
            cx,
        );
        let scrolled = window.find(("metadata", 8usize)).bounds();
        assert!(scrolled.top() < producer.top());
    })
    .unwrap();
}

#[gpui_kit::test]
fn exit_button_removes_the_viewer_window(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    cx.update(super::install_lifecycle);
    let (handle, _view) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Viewer::new(window, cx))
        })
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| window.click("exit", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(cx.windows().is_empty()));
}

#[gpui_kit::test]
fn quit_action_removes_the_viewer_window(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    cx.update(super::install_lifecycle);
    let (handle, _view) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let v = Viewer::new(window, cx);
                v.focus.focus(window, cx);
                v
            })
        })
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.draw(cx).clear(cx);
        window.dispatch_action(Box::new(super::Quit), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(cx.windows().is_empty()));
}
