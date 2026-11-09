use super::{Field, RecorderApp};
use core::prelude::rust_2024::test;
use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size};

fn toggle_measurement(cx: &mut TestAppContext, handle: gpui_kit::AnyWindowHandle, query: &str) {
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
}

#[gpui_kit::test]
fn measurement_combobox_toggles_by_identity_and_records_only_checked_items(
    cx: &mut TestAppContext,
) {
    use kittx::{datastore::Signal, obd::Pid};
    cx.update(gpui_kit::init);
    let (handle, app) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1100.), px(900.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| RecorderApp::new(window, cx)),
        )
        .unwrap()
    });
    let combo_id = cx.update(|cx| ("multi-combo-box", app.read(cx).measurements.entity_id()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            app.read(cx).settings(cx).unwrap().selected,
            [4, 5, 11, 12, 13, 15, 17, 256].into()
        );
        window.click("clear-signals", cx);
        assert!(
            app.read(cx)
                .settings(cx)
                .unwrap()
                .config()
                .unwrap()
                .signals
                .is_empty()
        );
        assert_eq!(app.read(cx).dtcs, [true; 3]);
    })
    .unwrap();
    toggle_measurement(cx, handle, "Engine RPM");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(combo_id).expanded(), Some(true));
        assert_eq!(app.read(cx).settings(cx).unwrap().selected, [0x0c].into());
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    toggle_measurement(cx, handle, "Vehicle speed (km/h)");
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            app.read(cx).settings(cx).unwrap().selected,
            [0x0c, 0x0d].into()
        );
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    toggle_measurement(cx, handle, "Engine RPM");
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(app.read(cx).settings(cx).unwrap().selected, [0x0d].into());
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    // Search the measurement name, then ensure the checked item still means 0161.
    toggle_measurement(cx, handle, "Driver demand engine torque");
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            app.read(cx).settings(cx).unwrap().selected,
            [0x0d, 0x61].into()
        );
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.simulate_window_resize(handle, size(px(820.), px(900.)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panel = window.find("checklist").bounds();
        let bounds = window.find("measurements").bounds();
        assert!(bounds.left() >= panel.left() && bounds.right() <= panel.right());
        // An unmatched search changes the visible options, never the checked values.
        window.click("measurements", cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
        window.input("not a measurement", cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let settings = app.read(cx).settings(cx).unwrap();
        assert_eq!(settings.selected, [0x0d, 0x61].into());
        assert_eq!(
            settings
                .config()
                .unwrap()
                .signals
                .iter()
                .map(|selection| selection.signal)
                .collect::<Vec<_>>(),
            [Signal::Pid(Pid::VehicleSpeed), Signal::RawPid(0x61)]
        );
        if cfg!(feature = "serial") {
            window.click("serial", cx);
            app.update(cx, |app, cx| {
                app.serial_port.update(cx, |state, cx| {
                    state.set_items(
                        crate::serial_ports::SerialOptions::new(vec!["COM7".into()]),
                        window,
                        cx,
                    );
                    state.set_selected_value(&"COM7".into(), window, cx);
                })
            });
            assert_eq!(
                app.read(cx).settings(cx).unwrap().selected,
                [0x0d, 0x61].into()
            );
        }
        window.click("start", cx);
        assert!(app.read(cx).choosing);
        window.render_frame(cx);
        window.click("measurements", cx);
        assert_eq!(window.find(combo_id).expanded(), Some(false));
        window.click("clear-signals", cx);
        assert_eq!(
            app.read(cx).settings(cx).unwrap().selected,
            [0x0d, 0x61].into()
        );
        assert!(app.read(cx).job.is_none());
    })
    .unwrap();
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(!app.read(cx).choosing);
        assert_eq!(
            app.read(cx).settings(cx).unwrap().selected,
            [0x0d, 0x61].into()
        );
        window.click("exit", cx);
    })
    .unwrap();
}

#[cfg(feature = "serial")]
fn configured_baud(app: &RecorderApp, cx: &gpui_kit::App) -> Result<u32, String> {
    let kittx::TransportConfig::Serial(config) = app.settings(cx)?.transport else {
        panic!("serial expected");
    };
    Ok(config.baud_rate)
}

#[gpui_kit::test]
#[cfg(feature = "serial")]
fn baud_combobox_defaults_presets_custom_validation_and_locking(cx: &mut TestAppContext) {
    use crate::serial_ports::SerialOptions;
    cx.update(gpui_kit::init);
    let (handle, app) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1100.), px(900.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| RecorderApp::new(window, cx)),
        )
        .unwrap()
    });
    let baud_id = cx.update(|cx| ("select", app.read(cx).baud_rate.entity_id()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("serial", cx);
        app.update(cx, |app, cx| {
            app.serial_port.update(cx, |state, cx| {
                state.set_items(SerialOptions::new(vec!["COM7".into()]), window, cx);
                state.set_selected_value(&"COM7".into(), window, cx);
            });
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(configured_baud(app.read(cx), cx).unwrap(), 38400);
    })
    .unwrap();
    choose_option(cx, handle, "baud-rate", "115200");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(configured_baud(app.read(cx), cx).unwrap(), 115200);
        // Unconfirmed search text must not replace the selected rate.
        window.click("baud-rate", cx);
        window.input("123457", cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(configured_baud(app.read(cx), cx).unwrap(), 115200));
    choose_option(cx, handle, "baud-rate", "123457");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(configured_baud(app.read(cx), cx).unwrap(), 123457);
        window.click("tcp", cx);
        window.click("serial", cx);
        window.render_frame(cx);
        assert_eq!(configured_baud(app.read(cx), cx).unwrap(), 123457);
        app.update(cx, |app, cx| {
            app.choosing = true;
            cx.notify();
        });
        window.render_frame(cx);
        window.click("baud-rate", cx);
        assert_eq!(window.find(baud_id).expanded(), Some(false));
        app.update(cx, |app, cx| {
            app.choosing = false;
            cx.notify();
        });
        for invalid in ["0", "-1", "9600.5", "4294967296", "not-a-baud", ""] {
            app.update(cx, |app, cx| {
                app.baud_rate.update(cx, |state, cx| {
                    let mut options = SerialOptions::baud_rates();
                    options.retain_value(invalid);
                    state.set_items(options, window, cx);
                    state.set_selected_value(&invalid.to_owned(), window, cx);
                })
            });
            assert_eq!(
                configured_baud(app.read(cx), cx).unwrap_err(),
                "Baud rate must be a positive integer"
            );
            assert!(app.read(cx).job.is_none());
        }
        app.update(cx, |app, cx| {
            app.baud_rate.update(cx, |state, cx| {
                state.set_selected_value(&"38400".into(), window, cx)
            })
        });
        window.render_frame(cx);
        window.within(baud_id).click("clean", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(configured_baud(app.read(cx), cx).is_err());
        window.click("exit", cx);
    })
    .unwrap();
}

#[gpui_kit::test]
#[cfg(feature = "serial")]
fn serial_combobox_selects_detected_or_custom_ports_and_preserves_selection(
    cx: &mut TestAppContext,
) {
    use crate::serial_ports::SerialOptions;
    use kittx::TransportConfig;
    cx.update(gpui_kit::init);
    let (handle, app) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1100.), px(900.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| RecorderApp::new(window, cx)),
        )
        .unwrap()
    });
    let port_id = cx.update(|cx| ("select", app.read(cx).serial_port.entity_id()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("serial", cx);
        app.update(cx, |app, cx| {
            app.serial_port.update(cx, |state, cx| {
                state.set_items(
                    SerialOptions::new(vec!["/dev/cu.usbserial-test".into(), "COM7".into()]),
                    window,
                    cx,
                )
            });
            cx.notify();
        });
        window.render_frame(cx);
        assert!(app.read(cx).settings(cx).is_err());
        assert!(app.read(cx).job.is_none());
    })
    .unwrap();
    choose_option(cx, handle, "serial-port", "/dev/cu.usbserial-test");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let TransportConfig::Serial(config) = app.read(cx).settings(cx).unwrap().transport else {
            panic!("serial expected");
        };
        assert_eq!(config.path.to_str(), Some("/dev/cu.usbserial-test"));
        assert_eq!(config.baud_rate, 38400);
        window.click("serial-port", cx);
        window.input("unconfirmed-port", cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            app.read(cx)
                .serial_port
                .read(cx)
                .selected_value()
                .map(String::as_str),
            Some("/dev/cu.usbserial-test")
        );
        // With no detected ports, manual entry remains available.
        app.update(cx, |app, cx| {
            app.serial_port.update(cx, |state, cx| {
                state.set_items(SerialOptions::new(Vec::new()), window, cx);
                state.set_selected_index(None, window, cx);
            });
            cx.notify();
        });
    })
    .unwrap();
    choose_option(cx, handle, "serial-port", "COM74");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("refresh-serial-ports", cx);
        window.render_frame(cx);
        assert_eq!(
            app.read(cx)
                .serial_port
                .read(cx)
                .selected_value()
                .map(String::as_str),
            Some("COM74")
        );
        window.click("tcp", cx);
        window.click("serial", cx);
        window.render_frame(cx);
        let TransportConfig::Serial(config) = app.read(cx).settings(cx).unwrap().transport else {
            panic!("serial expected");
        };
        assert_eq!(config.path.to_str(), Some("COM74"));
        app.update(cx, |app, cx| {
            app.choosing = true;
            cx.notify();
        });
        window.render_frame(cx);
        window.click("serial-port", cx);
        assert_eq!(window.find(port_id).expanded(), Some(false));
        window.click("refresh-serial-ports", cx);
        assert!(app.read(cx).job.is_none());
        app.update(cx, |app, cx| {
            app.choosing = false;
            cx.notify();
        });
        window.render_frame(cx);
        window.within(port_id).click("clean", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(app.read(cx).settings(cx).is_err());
        window.click("exit", cx);
    })
    .unwrap();
}

fn choose_option(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    field: &'static str,
    value: &str,
) {
    cx.update_window(handle, |_, window, cx| {
        window.click(field, cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
        window.input(value, cx);
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
fn accordions_and_vehicle_selection_metadata(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (handle, app) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1100.), px(900.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| RecorderApp::new(window, cx)),
        )
        .unwrap()
    });
    let brand_id = cx.update(|cx| ("select", app.read(cx).vehicle_brand.entity_id()));
    let model_id = cx.update(|cx| ("select", app.read(cx).vehicle_model.entity_id()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let settings = app.read(cx).settings(cx).unwrap();
        assert!(settings.vehicle.make.is_none());
        assert!(settings.vehicle.model.is_none());
        window.click(("trigger", 0usize), cx);
        assert!(!app.read(cx).settings_sections.contains(&0));
        assert!(app.read(cx).settings_sections.contains(&1));
        window.click(("trigger", 1usize), cx);
        window.click(("trigger", 2usize), cx);
        assert_eq!(app.read(cx).settings_sections, [2].into_iter().collect());
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vehicle-model", cx);
        assert_eq!(window.find(model_id).expanded(), Some(false));
    })
    .unwrap();

    choose_option(cx, handle, "vehicle-brand", "Volkswagen");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(model_id).expanded(), Some(false));
        assert_eq!(app.read(cx).selected_brand.as_deref(), Some("Volkswagen"));
    })
    .unwrap();
    choose_option(cx, handle, "vehicle-model", "Golf");
    cx.update(|cx| {
        let settings = app.read(cx).settings(cx).unwrap();
        assert_eq!(
            settings.vehicle.make.as_ref().unwrap(),
            &kittx::datastore::MetadataValue::caller("Volkswagen")
        );
        assert_eq!(
            settings.config().unwrap().vehicle.model.unwrap(),
            kittx::datastore::MetadataValue::caller("Golf")
        );
    });

    // Typing a search and cancelling must not replace committed metadata.
    cx.update_window(handle, |_, window, cx| {
        window.click("vehicle-brand", cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
        window.input("Unconfirmed brand", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let settings = app.read(cx).settings(cx).unwrap();
        assert_eq!(settings.vehicle.make.unwrap().value, "Volkswagen");
        assert_eq!(settings.vehicle.model.unwrap().value, "Golf");
    });

    choose_option(cx, handle, "vehicle-brand", "Toyota");
    cx.update(|cx| assert!(app.read(cx).settings(cx).unwrap().vehicle.model.is_none()));
    choose_option(cx, handle, "vehicle-model", "Corolla");
    cx.update(|cx| {
        assert_eq!(
            app.read(cx)
                .settings(cx)
                .unwrap()
                .vehicle
                .model
                .unwrap()
                .value,
            "Corolla"
        )
    });
    choose_option(cx, handle, "vehicle-brand", "Custom historic brand");
    cx.update(|cx| assert!(app.read(cx).settings(cx).unwrap().vehicle.model.is_none()));
    choose_option(cx, handle, "vehicle-model", "Custom model 1960");
    cx.update(|cx| {
        let settings = app.read(cx).settings(cx).unwrap();
        assert_eq!(
            settings.vehicle.make.unwrap().value,
            "Custom historic brand"
        );
        assert_eq!(settings.vehicle.model.unwrap().value, "Custom model 1960");
    });
    cx.simulate_window_resize(handle, size(px(820.), px(900.)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panel = window.find("settings").bounds();
        for field in ["vehicle-brand", "vehicle-model"] {
            let bounds = window.find(field).bounds();
            assert!(bounds.left() >= panel.left() && bounds.right() <= panel.right());
        }
        app.update(cx, |app, cx| {
            app.choosing = true;
            cx.notify();
        });
        window.render_frame(cx);
        window.click("vehicle-brand", cx);
        assert_eq!(window.find(brand_id).expanded(), Some(false));
        window.click("vehicle-model", cx);
        assert_eq!(window.find(model_id).expanded(), Some(false));
        app.update(cx, |app, cx| {
            app.choosing = false;
            cx.notify();
        });
        window.render_frame(cx);
        window.within(brand_id).click("clean", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let settings = app.read(cx).settings(cx).unwrap();
        assert!(settings.vehicle.make.is_none());
        assert!(settings.vehicle.model.is_none());
        window.click("vehicle-model", cx);
        assert_eq!(window.find(model_id).expanded(), Some(false));
        window.click("theme", cx);
        assert_eq!(app.read(cx).settings_sections, [2].into_iter().collect());
        window.click("exit", cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn modes_checklist_themes_save_cancel_and_validation(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    let (handle, app) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1100.), px(900.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| RecorderApp::new(window, cx)),
        )
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.draw(cx).clear(cx);
        assert!(cx.theme().is_dark());
        assert!(app.read(cx).settings(cx).unwrap().duration.is_none());
        window.click("timed-mode", cx);
        assert_eq!(
            app.read(cx)
                .settings(cx)
                .unwrap()
                .duration
                .unwrap()
                .as_secs(),
            300
        );
        window.click("manual-mode", cx);
        assert!(app.read(cx).settings(cx).unwrap().duration.is_none());
        window.click(("dtc", 0usize), cx);
        assert!(!app.read(cx).dtcs[0]);
        window.click(("dtc", 0usize), cx);
        assert!(app.read(cx).dtcs[0]);
        window.click("all-signals", cx);
        assert_eq!(app.read(cx).measurements.read(cx).selection().len(), 248);
        window.click("clear-signals", cx);
        assert!(app.read(cx).measurements.read(cx).selection().is_empty());
        window.click("decoded", cx);
        assert_eq!(app.read(cx).measurements.read(cx).selection().len(), 8);
        window.click("theme", cx);
        assert!(!cx.theme().is_dark());
        window.click("theme", cx);
        assert!(cx.theme().is_dark());
        window.click("start", cx);
        assert!(app.read(cx).choosing);
        assert!(app.read(cx).job.is_none());
    })
    .unwrap();
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    cx.update(|cx| {
        let app = app.read(cx);
        assert!(!app.choosing);
        assert!(app.job.is_none());
        assert!(app.message.contains("cancelled"));
    });
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.measurements
                .update(cx, |state, cx| state.clear_selection(cx));
            app.dtcs = [false; 3];
            cx.notify();
        });
        window.click("start", cx);
        assert!(!app.read(cx).choosing);
        assert!(app.read(cx).message.contains("Select at least one"));
        app.update(cx, |app, cx| {
            app.profile = true;
            app.fields[&Field::Request]
                .update(cx, |input, cx| input.set_value("1902FF", window, cx));
            cx.notify();
        });
        let settings = app.read(cx).settings(cx).unwrap();
        assert!(settings.config().unwrap().signals.is_empty());
        assert_eq!(settings.profile.unwrap().request, vec![0x19, 2, 0xff]);
        app.update(cx, |app, cx| {
            app.timed = true;
            app.fields[&Field::Duration].update(cx, |input, cx| input.set_value("0", window, cx));
        });
        assert!(app.read(cx).settings(cx).is_err());
        app.update(cx, |app, cx| {
            let mut status = crate::recording::SessionControl::new("trip.kittx".into()).snapshot();
            status.phase = crate::recording::Phase::Failed;
            status.message = "Capture failed; inspect the diagnostic log".into();
            status.last_error = Some("NO DATA while requesting VIN; ".repeat(100));
            status.diagnostics_path = Some("directory with spaces/trip.kittx.log".into());
            app.status = Some(status);
            cx.notify();
        });
        // Long diagnostics must wrap and scroll without hiding the controls in
        // either theme. The actual log retains the full error and wire evidence.
        for _ in 0..2 {
            window.render_frame(cx);
            let footer = window.find("recording-status").bounds();
            assert!(footer.size.height <= px(220.));
            assert!(footer.left() >= px(0.) && footer.right() <= px(1100.));
            let settings = window.find("settings").bounds();
            assert!(settings.bottom() <= footer.top());
            window.click("theme", cx);
        }
        window.click("exit", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(cx.windows().is_empty()));
}

#[gpui_kit::test]
fn save_normalizes_extension_and_exit_waits_for_active_worker(cx: &mut TestAppContext) {
    active_worker_close(cx, false);
}

#[gpui_kit::test]
fn hosted_exit_waits_for_active_worker_and_preserves_host(cx: &mut TestAppContext) {
    active_worker_close(cx, true);
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

fn active_worker_close(cx: &mut TestAppContext, hosted: bool) {
    use std::{
        net::TcpListener,
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };
    let directory =
        std::env::temp_dir().join(format!("kittx-recorder-ui-{}-{hosted}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let chosen = directory.join("capture.sqlite");
    let saved = directory.join("capture.kittx");
    assert!(!saved.exists());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let (accepted_tx, accepted_rx) = mpsc::sync_channel(1);
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        accepted_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(stream);
    });
    cx.update(gpui_kit::init);
    let host = hosted.then(|| {
        cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| TestHost))
                .unwrap()
                .0
        })
    });
    let (handle, app) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1100.), px(900.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| RecorderApp::new(window, cx)),
        )
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.fields[&Field::Port].update(cx, |input, cx| {
                input.set_value(port.to_string(), window, cx)
            });
        });
        window.click("start", cx);
    })
    .unwrap();
    cx.simulate_new_path_selection(|_| Some(chosen));
    cx.run_until_parked();
    accepted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    cx.update_window(handle, |_, window, cx| {
        assert!(app.read(cx).active());
        assert_eq!(app.read(cx).status.as_ref().unwrap().path, saved);
        window.click("exit", cx);
        assert!(app.read(cx).close_after_stop);
        assert!(app.read(cx).closing);
        assert!(
            app.read(cx).job.as_ref().unwrap().snapshot().phase
                == crate::recording::Phase::Stopping
        );
    })
    .unwrap();
    cx.update(|cx| assert_eq!(cx.windows().len(), if hosted { 2 } else { 1 }));
    release_tx.send(()).unwrap();
    server.join().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        if cx.update(|cx| cx.windows() == host.into_iter().collect::<Vec<_>>()) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "window did not close after worker completion"
        );
        thread::sleep(Duration::from_millis(1));
    }
    let doc = kittx_viewer::document::Document::open(saved).unwrap();
    assert_eq!(doc.overview.session.status, "start_failed");
    drop(doc);
    std::fs::remove_dir_all(directory).unwrap();
}
