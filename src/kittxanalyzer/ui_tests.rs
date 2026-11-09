use super::{Analyzer, Recording, Selection};
use crate::ai::{AiService, AnalysisError, AnalysisReport};
use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size};
use std::sync::{Arc, Mutex};

const TEST_REPORT: &str = "AI feedback: P0123 was observed. Check the throttle position sensor circuit before drawing a conclusion.";
const MARKDOWN_REPORT: &str = "# ECU analysis\n\n**Speed:** 45 km/h\n\n- P0123 was observed.\n- Coolant: 85 °C\n\n| Signal | Value |\n| --- | --- |\n| Speed | 45 km/h |\n| Coolant | 85 °C |\n\n## Verification\n\nCheck the `throttle` circuit before replacing parts.\n\n```text\nP0123 observed at 1000 us\n```\n";
type Requests = Arc<Mutex<Vec<(AiService, String)>>>;

#[path = "../../tests/common/viewer_fixture.rs"]
mod fixture;

fn open_app(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<Analyzer>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        super::install_lifecycle(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1280.), px(860.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let mut analyzer = Analyzer::new(window, cx);
                    analyzer.request_analysis = Arc::new(|_, _| {
                        Ok(AnalysisReport {
                            text: TEST_REPORT.into(),
                            incomplete: false,
                        })
                    });
                    analyzer
                })
            },
        )
        .unwrap()
    })
}

fn capture_requests(cx: &mut TestAppContext, app: &gpui_kit::Entity<Analyzer>) -> Requests {
    let requests = Requests::default();
    let captured = Arc::clone(&requests);
    cx.update(|cx| {
        app.update(cx, |app, _| {
            app.request_analysis = Arc::new(move |service, prompt| {
                captured.lock().unwrap().push((service, prompt.into()));
                Ok(AnalysisReport {
                    text: TEST_REPORT.into(),
                    incomplete: false,
                })
            });
        })
    });
    requests
}

fn choose_ai_service(cx: &mut TestAppContext, handle: gpui_kit::AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("ai-service", cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn ai_service_choices_and_header_layout(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    cx.update(|cx| assert!(app.read(cx).ai_service.read(cx).selected_value().is_none()));
    for expected in ["Anthropic", "OpenAI", "openrouter.ai"] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("ai-service", cx);
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(
                app.read(cx).ai_service.read(cx).selected_value(),
                Some(&expected)
            );
        });
    }
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("ai-service", cx);
        window.press("up", cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    for width in [1280., 760.] {
        cx.simulate_window_resize(handle, size(px(width), px(860.)));
        for _ in 0..2 {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                let open = window.find("open").bounds();
                let service = window.find("ai-service").bounds();
                let theme = window.find("theme").bounds();
                let exit = window.find("exit").bounds();
                assert!(open.right() <= service.left());
                assert!(service.right() <= theme.left());
                assert!(theme.right() <= exit.left());
                assert_eq!(service.top(), exit.top());
                assert!(exit.right() > px(width - 20.) && exit.right() <= px(width));
                assert_eq!(
                    app.read(cx).ai_service.read(cx).selected_value(),
                    Some(&"openrouter.ai")
                );
                window.click("theme", cx);
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn empty_state_theme_toggle_disabled_analyze_picker_cancel_and_exit(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(cx.theme().is_dark());
        assert!(!app.read(cx).can_analyze(cx));
        window.click("analyze", cx);
        assert!(app.read(cx).report.is_none());
        window.click("theme", cx);
        assert!(!cx.theme().is_dark());
        window.click("theme", cx);
        assert!(cx.theme().is_dark());
        window.click("open", cx);
        assert!(app.read(cx).choosing);
    })
    .unwrap();
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && !options.directories && !options.multiple);
        None
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(!app.read(cx).locked());
        assert!(app.read(cx).recording.is_none());
        window.render_frame(cx);
        window.click("exit", cx);
    })
    .unwrap();
    cx.update(|cx| assert!(cx.windows().is_empty()));
}

#[gpui_kit::test]
fn file_picker_selected_evidence_ai_report_and_both_theme_layouts(cx: &mut TestAppContext) {
    let f = fixture::Fixture::new();
    f.sample(1, 45.);
    f.scan(1, "ok", Some(1));
    let before = std::fs::read(&f.path).unwrap();
    let (handle, app) = open_app(cx);
    let requests = capture_requests(cx, &app);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("open", cx);
    })
    .unwrap();
    cx.simulate_path_prompt_response(|_| Some(vec![f.path.clone()]));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(app.read(cx).recording.as_ref().unwrap().path, f.path);
        assert_eq!(app.read(cx).selected, Selection::default());
        assert!(!app.read(cx).can_analyze(cx));
        window.click(("signal", 0usize), cx);
        assert_eq!(app.read(cx).selected.series.len(), 1);
        window.click(("dtc", 0usize), cx);
        assert_eq!(app.read(cx).selected.dtcs.len(), 1);
        assert!(!app.read(cx).can_analyze(cx));
        window.render_frame(cx);
        window.click("analyze", cx);
        window.dispatch_action(Box::new(super::Analyze), cx);
        assert!(!app.read(cx).analyzing);
        assert!(app.read(cx).report.is_none());
    })
    .unwrap();
    cx.run_until_parked();
    assert!(requests.lock().unwrap().is_empty());
    choose_ai_service(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(app.read(cx).can_analyze(cx));
        window.click("analyze", cx);
        assert!(app.read(cx).analyzing);
        assert!(!app.read(cx).can_analyze(cx));
        window.click("clear", cx);
        assert_eq!(app.read(cx).selected.series.len(), 1);
        window.dispatch_action(Box::new(super::Analyze), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let report = app.read(cx).report.as_ref().unwrap();
        assert_eq!(report, TEST_REPORT);
        assert!(
            !report.contains("Selected recording evidence")
                && !report.contains("measurement_series")
        );
        assert!(
            app.read(cx)
                .status
                .contains("Analysis from Anthropic is ready")
        );
        assert!(!app.read(cx).analyzing);
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, AiService::Anthropic);
        assert!(requests[0].1.contains("\"sample_count\": 1"));
        assert!(requests[0].1.contains("\"value\": 45.0"));
        assert_eq!(
            &requests[0].1,
            &app.read(cx)
                .recording
                .as_ref()
                .unwrap()
                .analysis_prompt(&app.read(cx).selected)
                .unwrap()
        );
        window.click(("signal", 0usize), cx);
        assert!(app.read(cx).report.is_none());
        window.click("analyze", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(app.read(cx).report.as_deref(), Some(TEST_REPORT));
        assert!(
            requests.lock().unwrap()[1]
                .1
                .contains("\"measurement_series\": []")
        );
        window.click("clear", cx);
        assert_eq!(app.read(cx).selected, Selection::default());
        assert!(app.read(cx).report.is_none());
        assert!(!app.read(cx).can_analyze(cx));
        window.click("select-all", cx);
        assert_eq!(
            app.read(cx).selected,
            Selection::all(app.read(cx).recording.as_ref().unwrap())
        );
        window.click("analyze", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.simulate_window_resize(handle, size(px(800.), px(650.)));
    for _ in 0..2 {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let left = window.find("data-panel").bounds();
            let right = window.find("report-panel").bounds();
            assert!(left.right() <= right.left());
            assert!(right.right() <= px(801.));
            assert!(right.size.width > px(250.));
            let signal = window.find(("signal", 0usize)).bounds();
            assert!(signal.left() >= left.left() && signal.right() <= left.right());
            assert!(window.find("analyze").visible());
            assert!(window.find("exit").visible());
            assert!(app.read(cx).report.as_ref().unwrap().contains("P0123"));
            window.click("theme", cx);
        })
        .unwrap();
    }
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
}

#[gpui_kit::test]
fn failed_load_preserves_previous_file_and_successful_reopen_resets_selection(
    cx: &mut TestAppContext,
) {
    let f = fixture::Fixture::new();
    f.sample(1, 30.);
    f.scan(1, "ok", Some(0));
    let (handle, app) = open_app(cx);
    choose_ai_service(cx, handle);
    cx.update(|cx| app.update(cx, |app, cx| app.load(f.path.clone(), cx)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("select-all", cx);
        window.click("analyze", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(app.read(cx).report.is_some());
        app.update(cx, |app, cx| app.load(f.path.with_extension("txt"), cx));
        assert!(app.read(cx).opening);
        assert!(!app.read(cx).can_analyze(cx));
        window.render_frame(cx);
        window.click("clear", cx);
        assert!(!app.read(cx).selected.series.is_empty());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let analyzer = app.read(cx);
        assert!(analyzer.status.contains("Cannot open recording"));
        assert_eq!(analyzer.recording.as_ref().unwrap().path, f.path);
        assert!(analyzer.report.is_some());
        assert!(analyzer.can_analyze(cx));
    });
    let next = fixture::Fixture::new();
    cx.update(|cx| app.update(cx, |app, cx| app.load(next.path.clone(), cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        let analyzer = app.read(cx);
        assert_eq!(analyzer.recording.as_ref().unwrap().path, next.path);
        assert!(analyzer.report.is_none());
        assert_eq!(analyzer.selected, Selection::default());
        assert!(!analyzer.can_analyze(cx));
    });
    // Exercise the same metadata path independently of the UI load task.
    assert!(Recording::open(next.path.clone()).unwrap().dtcs.is_empty());
}

#[gpui_kit::test]
fn prompt_generation_failure_unlocks_controls_without_a_report(cx: &mut TestAppContext) {
    let f = fixture::Fixture::new();
    f.sample(1, 30.);
    f.scan(1, "ok", Some(0));
    let (handle, app) = open_app(cx);
    let requests = capture_requests(cx, &app);
    choose_ai_service(cx, handle);
    cx.update(|cx| app.update(cx, |app, cx| app.load(f.path.clone(), cx)));
    cx.run_until_parked();
    f.db.execute("UPDATE session SET id='replaced'", [])
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("select-all", cx);
        window.click("analyze", cx);
        assert!(app.read(cx).analyzing);
        window.click("clear", cx);
        assert!(!app.read(cx).selected.series.is_empty());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let analyzer = app.read(cx);
        assert!(!analyzer.analyzing);
        assert!(analyzer.can_analyze(cx));
        assert!(analyzer.report.is_none());
        assert!(analyzer.status.contains("Cannot generate prompt"));
    });
    assert!(requests.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn analyze_uses_the_selected_provider_and_hides_the_prompt(cx: &mut TestAppContext) {
    let f = fixture::Fixture::new();
    f.sample(1, 45.);
    f.scan(1, "ok", Some(1));
    let (handle, app) = open_app(cx);
    let requests = capture_requests(cx, &app);
    cx.update(|cx| app.update(cx, |app, cx| app.load(f.path.clone(), cx)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("select-all", cx);
    })
    .unwrap();
    for (index, service) in AiService::ALL.into_iter().enumerate() {
        choose_ai_service(cx, handle);
        cx.update_window(handle, |_, window, cx| {
            assert!(app.read(cx).report.is_none());
            assert_eq!(app.read(cx).selected_service(cx), Some(service));
            window.render_frame(cx);
            window.click("analyze", cx);
            assert!(app.read(cx).analyzing);
            assert!(app.read(cx).report.is_none());
            window.click("open", cx);
            assert!(!app.read(cx).choosing);
            window.dispatch_action(Box::new(super::Analyze), cx);
            window.click("theme", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let analyzer = app.read(cx);
            assert_eq!(analyzer.report.as_deref(), Some(TEST_REPORT));
            assert!(analyzer.status.contains(service.label()));
            let sent = requests.lock().unwrap();
            assert_eq!(sent.len(), index + 1);
            assert_eq!(sent[index].0, service);
            assert_eq!(
                sent[index].1,
                analyzer
                    .recording
                    .as_ref()
                    .unwrap()
                    .analysis_prompt(&analyzer.selected)
                    .unwrap()
            );
            assert_ne!(analyzer.report.as_ref().unwrap(), &sent[index].1);
            assert!(analyzer.can_analyze(cx));
        });
    }
}

#[gpui_kit::test]
fn ai_failures_unlock_controls_and_incomplete_feedback_is_visible(cx: &mut TestAppContext) {
    let f = fixture::Fixture::new();
    f.sample(1, 45.);
    let (handle, app) = open_app(cx);
    choose_ai_service(cx, handle);
    cx.update(|cx| app.update(cx, |app, cx| app.load(f.path.clone(), cx)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("select-all", cx);
    })
    .unwrap();
    for error in [
        AnalysisError::MissingApiKey("ANTHROPIC_API_KEY"),
        AnalysisError::Timeout,
        AnalysisError::Http(429),
        AnalysisError::Connection,
        AnalysisError::Provider,
        AnalysisError::InvalidResponse,
        AnalysisError::EmptyResponse,
    ] {
        let expected = error.to_string();
        cx.update(|cx| {
            app.update(cx, |app, _| {
                app.request_analysis = Arc::new(move |_, _| Err(error.clone()));
                app.report = Some("Old report that must be cleared".into());
            })
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("analyze", cx);
            assert!(app.read(cx).report.is_none());
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let analyzer = app.read(cx);
            assert!(!analyzer.analyzing);
            assert!(analyzer.can_analyze(cx));
            assert!(analyzer.report.is_none());
            assert!(analyzer.status.contains(&expected));
            assert!(analyzer.status.contains("Anthropic"));
            assert!(!analyzer.selected.series.is_empty());
        });
    }
    cx.update(|cx| {
        app.update(cx, |app, _| {
            app.request_analysis = Arc::new(|_, _| {
                Ok(AnalysisReport {
                    text: "Partial AI feedback".into(),
                    incomplete: true,
                })
            });
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("analyze", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(app.read(cx).report.as_deref(), Some("Partial AI feedback"));
        assert!(app.read(cx).status.contains("incomplete report"));
        assert!(app.read(cx).can_analyze(cx));
    });
}

#[gpui_kit::test]
fn save_report_cancel_export_and_errors_preserve_feedback_and_recording(cx: &mut TestAppContext) {
    let f = fixture::Fixture::new();
    f.sample(1, 45.);
    f.scan(1, "ok", Some(1));
    let before = std::fs::read(&f.path).unwrap();
    let (handle, app) = open_app(cx);
    cx.update_window(handle, |_, window, cx| {
        for report in [None, Some("".into()), Some(" \n\t".into())] {
            app.update(cx, |app, _| app.report = report);
            window.render_frame(cx);
            assert!(!app.read(cx).can_save_report());
            window.click("save-report", cx);
            window.dispatch_action(Box::new(super::SaveReport), cx);
            assert!(!app.read(cx).saving);
        }
        app.update(cx, |app, cx| {
            app.request_analysis = Arc::new(|_, _| {
                Ok(AnalysisReport {
                    text: MARKDOWN_REPORT.into(),
                    incomplete: false,
                })
            });
            app.load(f.path.clone(), cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    choose_ai_service(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("select-all", cx);
        window.click("analyze", cx);
        assert!(!app.read(cx).can_save_report());
        window.click("save-report", cx);
        assert!(!app.read(cx).saving);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(app.read(cx).report.as_deref(), Some(MARKDOWN_REPORT));
        assert!(app.read(cx).can_save_report());
        window.click("save-report", cx);
        assert!(app.read(cx).saving);
        assert!(!app.read(cx).can_analyze(cx));
        assert!(!app.read(cx).can_save_report());
        window.click("save-report", cx);
        window.dispatch_action(Box::new(super::SaveReport), cx);
        window.click("clear", cx);
        window.click("open", cx);
        assert!(!app.read(cx).choosing);
        assert!(!app.read(cx).selected.series.is_empty());
        window.click("theme", cx);
        assert!(!cx.theme().is_dark());
    })
    .unwrap();
    cx.simulate_new_path_selection(|directory| {
        assert_eq!(directory, f.path.parent().unwrap());
        None
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!app.read(cx).saving);
        assert!(app.read(cx).can_save_report());
        assert_eq!(app.read(cx).report.as_deref(), Some(MARKDOWN_REPORT));
        assert!(app.read(cx).status.contains("cancelled"));
    });

    let chosen = f.path.with_extension("");
    let saved = f.path.with_extension("md");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("save-report", cx);
    })
    .unwrap();
    cx.simulate_new_path_selection(|_| Some(chosen));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), MARKDOWN_REPORT);
    cx.update(|cx| {
        assert!(app.read(cx).can_save_report());
        assert_eq!(app.read(cx).report.as_deref(), Some(MARKDOWN_REPORT));
        assert!(app.read(cx).status.contains(&saved.display().to_string()));
        assert!(app.read(cx).status.starts_with("Saved report"));
    });

    // A failed write must retain the report and allow another attempt.
    let missing_parent = f.path.with_extension("missing").join("report.md");
    cx.update_window(handle, |_, window, cx| {
        window.click("save-report", cx);
    })
    .unwrap();
    cx.simulate_new_path_selection(|_| Some(missing_parent));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!app.read(cx).saving);
        assert!(app.read(cx).can_save_report());
        assert_eq!(app.read(cx).report.as_deref(), Some(MARKDOWN_REPORT));
        assert!(app.read(cx).status.starts_with("Cannot save report"));
    });

    // The exact filename selected in the dialog can be replaced on a later save.
    std::fs::write(&saved, "Old report with trailing data".repeat(20)).unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.click("save-report", cx);
    })
    .unwrap();
    cx.simulate_new_path_selection(|_| Some(saved.clone()));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), MARKDOWN_REPORT);
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
    for width in [1280., 760.] {
        cx.simulate_window_resize(handle, size(px(width), px(650.)));
        for _ in 0..2 {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                let panel = window.find("report-panel").bounds();
                let save = window.find("save-report").bounds();
                assert!(window.find("save-report").visible());
                assert!(save.left() >= panel.left() && save.right() <= panel.right());
                window.click("theme", cx);
            })
            .unwrap();
        }
    }
    cx.update_window(handle, |_, window, cx| {
        window.click("clear", cx);
        assert!(!app.read(cx).can_save_report());
    })
    .unwrap();
    std::fs::remove_file(saved).unwrap();
}

#[test]
fn markdown_export_extensions_and_existing_targets_do_not_modify_the_recording() {
    let f = fixture::Fixture::new();
    let directory = f.path.with_extension("report-test");
    std::fs::create_dir(&directory).unwrap();
    for name in ["report.md", "UPPER.MD", "report.markdown", "UPPER.MARKDOWN"] {
        let path = directory.join(name);
        assert_eq!(
            super::save_markdown(path.clone(), MARKDOWN_REPORT).unwrap(),
            path
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), MARKDOWN_REPORT);
    }
    let original = directory.join("report.txt");
    std::fs::write(&original, "Keep this original file").unwrap();
    // report.md already exists, so a normalized name must not overwrite it.
    assert!(super::save_markdown(original.clone(), "replacement").is_err());
    assert_eq!(
        std::fs::read_to_string(&original).unwrap(),
        "Keep this original file"
    );
    assert_eq!(
        std::fs::read_to_string(directory.join("report.md")).unwrap(),
        MARKDOWN_REPORT
    );
    assert!(super::save_markdown(directory.clone(), MARKDOWN_REPORT).is_err());
    assert!(super::save_markdown(std::path::PathBuf::new(), MARKDOWN_REPORT).is_err());
    let before = std::fs::read(&f.path).unwrap();
    let markdown = super::save_markdown(f.path.clone(), MARKDOWN_REPORT).unwrap();
    assert_eq!(markdown, f.path.with_extension("md"));
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
    assert_eq!(std::fs::read_to_string(&markdown).unwrap(), MARKDOWN_REPORT);
    std::fs::remove_file(markdown).unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui_kit::test]
fn long_checklist_and_report_scroll_independently(cx: &mut TestAppContext) {
    use gpui_kit::{ScrollDelta, point};
    let f = fixture::Fixture::new();
    f.sample(1, 40.);
    f.scan(1, "ok", Some(1));
    for index in 0..60 {
        let signal = format!("vendor:{index:03}");
        f.db.execute(
            "INSERT INTO signal_definition VALUES(?1,1,99,'unit','1',0)",
            [&signal],
        )
        .unwrap();
        f.db.execute(
            "INSERT INTO measurement VALUES(?1,1,1,?2,1000,1000,'number',42,NULL,'unit')",
            rusqlite::params![index + 100, signal],
        )
        .unwrap();
    }
    let (handle, app) = open_app(cx);
    cx.update(|cx| {
        app.update(cx, |app, _| {
            app.request_analysis = Arc::new(|_, _| {
                Ok(AnalysisReport {
                    text: format!("{MARKDOWN_REPORT}\n\n").repeat(64),
                    incomplete: false,
                })
            });
        })
    });
    choose_ai_service(cx, handle);
    cx.update(|cx| app.update(cx, |app, cx| app.load(f.path.clone(), cx)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!window.find(("dtc", 0usize)).visible());
        window.scroll(
            "data-panel",
            ScrollDelta::Pixels(point(px(0.), px(-100_000.))),
            cx,
        );
        window.render_frame(cx);
        assert!(app.read(cx).choices_scroll.offset().y < px(0.));
        assert!(window.find(("dtc", 0usize)).visible());
        window.click(("dtc", 0usize), cx);
        assert_eq!(app.read(cx).selected.dtcs.len(), 1);
        window.click("select-all", cx);
        window.click("analyze", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let choices_offset = app.read(cx).choices_scroll.offset();
        window.scroll(
            "report-panel",
            ScrollDelta::Pixels(point(px(0.), px(-100_000.))),
            cx,
        );
        window.render_frame(cx);
        assert!(app.read(cx).report_scroll.offset().y < px(0.));
        assert_eq!(app.read(cx).choices_scroll.offset(), choices_offset);
        assert!(window.find("analyze").visible());
        assert!(window.find("open").visible());
        window.click("analyze", cx);
        assert_eq!(app.read(cx).report_scroll.offset().y, px(0.));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(app.read(cx).report.is_some()));
}
