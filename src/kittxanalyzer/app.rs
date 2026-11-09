use crate::ui_theme::apply as apply_analyzer_theme;
use crate::{
    ai::{self, AiService, AnalysisError, AnalysisReport},
    model::{Recording, Selection, signal_label},
};
use gpui_kit::base::{Disableable, StyledExt};
use gpui_kit::component::{
    ActiveTheme, Theme, ThemeMode,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    resizable::{h_resizable, resizable_panel},
    scroll::ScrollableElement,
    select::{Select, SelectEvent, SelectState},
    text::TextView,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::{io::Write, path::PathBuf, sync::Arc};

type RequestAnalysis =
    dyn Fn(AiService, &str) -> Result<AnalysisReport, AnalysisError> + Send + Sync;

fn analyzer_button(id: impl Into<ElementId>) -> Button {
    Button::new(id).primary().outline().rounded(px(8.))
}

actions!(analyzer, [Open, Analyze, SaveReport, Quit]);

pub(crate) fn install_lifecycle(cx: &mut App) {
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}

fn exit(window: &mut Window, _: &mut App) {
    window.remove_window();
}

fn text(value: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .w_full()
        .flex_shrink_0()
        .whitespace_normal()
        .child(value.into())
}

fn panel(id: &'static str, child: impl IntoElement) -> impl IntoElement {
    div()
        .id(id)
        .size_full()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .child(child)
        .map(|panel| {
            #[cfg(feature = "ui-test")]
            {
                use gpui_kit::test::TestSupportExt;
                panel.test_support()
            }
            #[cfg(not(feature = "ui-test"))]
            {
                panel
            }
        })
}

fn save_markdown(chosen: PathBuf, report: &str) -> Result<PathBuf, String> {
    if chosen.file_name().is_none() || chosen.is_dir() {
        return Err("Choose a Markdown filename.".into());
    }
    let path = if chosen
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
        }) {
        chosen.clone()
    } else {
        chosen.with_extension("md")
    };
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    if path == chosen {
        // The native save dialog confirms replacement of the chosen file.
        options.create(true).truncate(true);
    } else {
        // A normalized filename was not confirmed by the dialog. Do not replace
        // an existing file under that different name.
        options.create_new(true);
    }
    options
        .open(&path)
        .and_then(|mut file| file.write_all(report.as_bytes()))
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

pub(crate) struct Analyzer {
    ai_service: Entity<SelectState<Vec<&'static str>>>,
    recording: Option<Recording>,
    selected: Selection,
    report: Option<String>,
    status: String,
    choosing: bool,
    opening: bool,
    analyzing: bool,
    saving: bool,
    request_analysis: Arc<RequestAnalysis>,
    focus: FocusHandle,
    choices_scroll: ScrollHandle,
    report_scroll: ScrollHandle,
}

impl Analyzer {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        apply_analyzer_theme(cx);
        let ai_service = cx.new(|cx| {
            SelectState::new(
                AiService::ALL
                    .into_iter()
                    .map(AiService::label)
                    .collect::<Vec<_>>(),
                None,
                window,
                cx,
            )
        });
        cx.subscribe(
            &ai_service,
            |this, _, _: &SelectEvent<Vec<&'static str>>, cx| {
                if !this.locked() {
                    this.selection_changed(cx);
                }
                cx.notify();
            },
        )
        .detach();
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            ai_service,
            recording: None,
            selected: Selection::default(),
            report: None,
            status: "Open a .kittx recording to choose data for analysis.".into(),
            choosing: false,
            opening: false,
            analyzing: false,
            saving: false,
            request_analysis: Arc::new(ai::analyze),
            focus,
            choices_scroll: ScrollHandle::new(),
            report_scroll: ScrollHandle::new(),
        }
    }

    fn locked(&self) -> bool {
        self.choosing || self.opening || self.analyzing || self.saving
    }

    fn selected_service(&self, cx: &App) -> Option<AiService> {
        let label = self.ai_service.read(cx).selected_value()?;
        AiService::ALL
            .into_iter()
            .find(|service| service.label() == *label)
    }

    fn can_analyze(&self, cx: &App) -> bool {
        self.recording.is_some()
            && self.selected_service(cx).is_some()
            && !self.locked()
            && (!self.selected.series.is_empty() || !self.selected.dtcs.is_empty())
    }

    fn can_save_report(&self) -> bool {
        !self.locked()
            && self
                .report
                .as_ref()
                .is_some_and(|report| !report.trim().is_empty())
    }

    fn save_report(&mut self, _: &SaveReport, _: &mut Window, cx: &mut Context<Self>) {
        if !self.can_save_report() {
            return;
        }
        let Some(report) = self.report.clone() else {
            return;
        };
        let directory = self
            .recording
            .as_ref()
            .and_then(|recording| recording.path.parent())
            .filter(|directory| !directory.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir()));
        let name = self
            .recording
            .as_ref()
            .and_then(|recording| recording.path.file_stem())
            .map(|stem| format!("{}-report.md", stem.to_string_lossy()))
            .unwrap_or_else(|| "analysis-report.md".into());
        self.saving = true;
        self.status = "Choose where to save the Markdown report.".into();
        let picker = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn(async move |this, cx| {
            let result = match picker.await {
                Ok(Ok(Some(path))) => {
                    cx.background_spawn(async move { save_markdown(path, &report).map(Some) })
                        .await
                }
                Ok(Ok(None)) => Ok(None),
                error => Err(format!("Save dialog failed: {error:?}")),
            };
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                this.status = match result {
                    Ok(Some(path)) => format!("Saved report to {}", path.display()),
                    Ok(None) => "Report save cancelled.".into(),
                    Err(error) => format!("Cannot save report: {error}"),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn selection_changed(&mut self, cx: &mut Context<Self>) {
        self.report = None;
        self.status = "Selection updated. Click Analyze to ask the selected AI service.".into();
        cx.notify();
    }

    fn open(&mut self, _: &Open, _: &mut Window, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        self.choosing = true;
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open .kittx recording".into()),
        });
        cx.spawn(async move |this, cx| {
            let result = picker.await;
            let _ = this.update(cx, |this, cx| {
                this.choosing = false;
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            this.load(path, cx);
                        }
                    }
                    Ok(Ok(None)) => (),
                    error => this.status = format!("File picker failed: {error:?}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn load(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        self.opening = true;
        self.status = format!("Opening {}…", path.display());
        let task = cx.background_spawn(async move { Recording::open(path) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.opening = false;
                match result {
                    Ok(recording) => {
                        this.status = format!(
                            "Loaded {} · {} measurement series · {} DTC items. Select what to analyze.",
                            recording.path.display(), recording.overview.series.len(), recording.dtcs.len(),
                        );
                        this.recording = Some(recording);
                        this.selected = Selection::default();
                        this.report = None;
                        this.choices_scroll.set_offset(point(px(0.), px(0.)));
                        this.report_scroll.set_offset(point(px(0.), px(0.)));
                    }
                    Err(error) => this.status = format!("Cannot open recording: {error}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn analyze(&mut self, _: &Analyze, _: &mut Window, cx: &mut Context<Self>) {
        if !self.can_analyze(cx) {
            return;
        }
        let Some(recording) = self.recording.clone() else {
            return;
        };
        let Some(service) = self.selected_service(cx) else {
            return;
        };
        let selection = self.selected.clone();
        let request_analysis = Arc::clone(&self.request_analysis);
        self.analyzing = true;
        self.report = None;
        self.report_scroll.set_offset(point(px(0.), px(0.)));
        self.status = format!(
            "Asking {} to analyze the selected recording data…",
            service.label()
        );
        let task = cx.background_spawn(async move {
            let prompt = recording
                .analysis_prompt(&selection)
                .map_err(|error| format!("Cannot generate prompt: {error}"))?;
            request_analysis(service, &prompt)
                .map_err(|error| format!("{}: {error}", service.label()))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.analyzing = false;
                match result {
                    Ok(report) => {
                        this.status = if report.incomplete {
                            format!("{} returned an incomplete report (output limit or interrupted generation).", service.label())
                        } else {
                            format!("Analysis from {} is ready.", service.label())
                        };
                        this.report = Some(report.text);
                    }
                    Err(error) => this.status = error,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for Analyzer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bg = cx.theme().background;
        let fg = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let busy = self.locked();
        let mut choices = div()
            .id("choices-scroll")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .w_full()
            .p_4()
            .flex()
            .flex_col()
            .gap_3();
        if let Some(recording) = &self.recording {
            let overview = &recording.overview;
            choices = choices
                .child(text("RECORDING").text_xs().text_color(muted))
                .child(
                    text(
                        recording
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                    )
                    .font_semibold(),
                )
                .child(text(format!("Session: {}", overview.session.id)).text_sm())
                .child(
                    text(format!(
                        "VIN: {}",
                        overview.session.vin.as_deref().unwrap_or("-")
                    ))
                    .text_sm(),
                )
                .child(text(format!("Status: {}", overview.session.status)).text_sm());
            for field in &overview.vehicle {
                choices = choices.child(
                    text(format!(
                        "{}: {}",
                        field.name,
                        field.value.as_deref().unwrap_or("-"),
                    ))
                    .text_sm(),
                );
            }
            choices = choices.child(
                text("ANALOG DATA / MEASUREMENTS")
                    .text_xs()
                    .text_color(muted),
            );
            if overview.series.is_empty() {
                choices = choices.child(
                    text("No measurement samples in this recording.")
                        .text_sm()
                        .text_color(muted),
                );
            }
            // Numeric series first; raw measurements remain selectable and are
            // explicitly labeled rather than presented as decoded analog values.
            for numeric in [true, false] {
                if !numeric && overview.series.iter().any(|series| !series.numeric) {
                    choices = choices.child(
                        text("RAW / UNDECODED MEASUREMENTS")
                            .text_xs()
                            .text_color(muted),
                    );
                }
                for (index, series) in overview
                    .series
                    .iter()
                    .enumerate()
                    .filter(|(_, series)| series.numeric == numeric)
                {
                    let label = format!(
                        "{} [{}] · {} · ECU {} ({}) · {} samples",
                        signal_label(&series.signal),
                        series.signal,
                        series.unit,
                        series.ecu_id,
                        series.sender,
                        series.count,
                    );
                    choices = choices.child(
                        Checkbox::new(("signal", index))
                            .label(label)
                            .w_full()
                            .min_w_0()
                            .flex_shrink_0()
                            .whitespace_normal()
                            .checked(self.selected.series.contains(&index))
                            .disabled(busy)
                            .on_click(cx.listener(move |this, checked, _, cx| {
                                if *checked {
                                    this.selected.series.insert(index);
                                } else {
                                    this.selected.series.remove(&index);
                                }
                                this.selection_changed(cx);
                            })),
                    );
                }
            }
            choices = choices.child(text("DIAGNOSTIC TROUBLE CODES").text_xs().text_color(muted));
            if recording.dtcs.is_empty() {
                choices = choices.child(
                    text("No DTC reads in this recording.")
                        .text_sm()
                        .text_color(muted),
                );
            }
            for (index, dtc) in recording.dtcs.iter().enumerate() {
                choices = choices.child(
                    Checkbox::new(("dtc", index))
                        .label(format!("{} · {} reads", dtc.label(), dtc.observations))
                        .w_full()
                        .min_w_0()
                        .flex_shrink_0()
                        .whitespace_normal()
                        .checked(self.selected.dtcs.contains(&index))
                        .disabled(busy)
                        .on_click(cx.listener(move |this, checked, _, cx| {
                            if *checked {
                                this.selected.dtcs.insert(index);
                            } else {
                                this.selected.dtcs.remove(&index);
                            }
                            this.selection_changed(cx);
                        })),
                );
            }
        } else {
            choices = choices
                .child(text("Choose recording data").font_semibold())
                .child(
                    text("Open a .kittx file to list its measurements and DTC observations here.")
                        .text_color(muted),
                );
        }
        let sidebar = div()
            .bg(cx.theme().sidebar)
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .p_4()
                    .bg(cx.theme().title_bar)
                    .border_b_1()
                    .border_color(border)
                    .child(text("Data to analyze").text_lg().font_semibold())
                    .child(
                        div()
                            .mt_3()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                analyzer_button("select-all")
                                    .label("Select all")
                                    .disabled(self.recording.is_none() || busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(recording) = &this.recording {
                                            this.selected = Selection::all(recording);
                                            this.selection_changed(cx);
                                        }
                                    })),
                            )
                            .child(
                                analyzer_button("clear")
                                    .label("Clear")
                                    .disabled(self.recording.is_none() || busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.selected = Selection::default();
                                        this.selection_changed(cx);
                                    })),
                            ),
                    ),
            )
            .child(
                choices
                    .overflow_y_scroll()
                    .track_scroll(&self.choices_scroll)
                    .vertical_scrollbar(&self.choices_scroll),
            )
            .child(
                div()
                    .p_4()
                    .border_t_1()
                    .border_color(border)
                    .flex_shrink_0()
                    .child(
                        text(format!(
                            "{} measurements · {} DTC items selected",
                            self.selected.series.len(),
                            self.selected.dtcs.len()
                        ))
                        .text_sm()
                        .text_color(muted),
                    )
                    .child(
                        analyzer_button("analyze")
                            .label(if self.analyzing {
                                "Analyzing…"
                            } else {
                                "Analyze"
                            })
                            .mt_3()
                            .w_full()
                            .disabled(!self.can_analyze(cx))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.analyze(&Analyze, window, cx)
                            })),
                    ),
            );
        let report_body = if let Some(report) = &self.report {
            TextView::markdown("report-markdown", report.clone())
                .text_sm()
                .w_full()
                .min_w_0()
                .scrollable(false)
                .into_any_element()
        } else {
            text(if self.analyzing {
                "Waiting for the AI service to return its analysis…"
            } else {
                "Your AI analysis report will appear here.\n\nSelect measurements or DTCs, choose an AI service, then click Analyze."
            })
                .text_color(muted)
                .into_any_element()
        };
        let report = div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .p_4()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(div().min_w_0().text_lg().font_semibold().child("Analysis report"))
                            .child(
                                analyzer_button("save-report")
                                    .label(if self.saving { "Saving…" } else { "Save report…" })
                                    .disabled(!self.can_save_report())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_report(&SaveReport, window, cx)
                                    })),
                            ),
                    )
                    .child(
                        text("Analyze sends only selected recording evidence to the chosen AI service.")
                            .text_sm()
                            .text_color(muted),
                    ),
            )
            .child(
                div()
                    .id("report-scroll")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .w_full()
                    .p_6()
                    .child(report_body)
                    .overflow_y_scroll()
                    .track_scroll(&self.report_scroll)
                    .vertical_scrollbar(&self.report_scroll),
            );
        let panels = h_resizable("analyzer-panels")
            .child(
                resizable_panel()
                    .size(px(400.))
                    .size_range(px(280.)..px(580.))
                    .child(panel("data-panel", sidebar)),
            )
            .child(resizable_panel().child(panel("report-panel", report)));
        div()
            .key_context("Analyzer")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::analyze))
            .on_action(cx.listener(Self::save_report))
            .on_action(|_: &Quit, window, cx| exit(window, cx))
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(bg)
            .text_color(fg)
            .child(
                div()
                    .p_3()
                    .border_b_1()
                    .border_color(border)
                    .bg(cx.theme().title_bar)
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .mr_3()
                            .font_semibold()
                            .text_lg()
                            .child("KittxAnalyzer"),
                    )
                    .child(
                        analyzer_button("open")
                            .label(if self.opening {
                                "Opening…"
                            } else {
                                "Open .kittx…"
                            })
                            .disabled(busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open(&Open, window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("ai-service")
                            .w(px(180.))
                            .min_w_0()
                            .flex_shrink_0()
                            .child(
                                Select::new(&self.ai_service)
                                    .disabled(busy)
                                    .w_full()
                                    .placeholder("AI service")
                                    .accessibility_label("AI service"),
                            )
                            .map(|field| {
                                #[cfg(feature = "ui-test")]
                                {
                                    use gpui_kit::test::TestSupportExt;
                                    field.test_support()
                                }
                                #[cfg(not(feature = "ui-test"))]
                                {
                                    field
                                }
                            }),
                    )
                    .child(
                        analyzer_button("theme")
                            .label(if cx.theme().is_dark() {
                                "Light theme"
                            } else {
                                "Dark theme"
                            })
                            .ml_auto()
                            .on_click(cx.listener(|_, _, window, cx| {
                                let mode = if cx.theme().is_dark() {
                                    ThemeMode::Light
                                } else {
                                    ThemeMode::Dark
                                };
                                Theme::change(mode, Some(window), cx);
                                apply_analyzer_theme(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        analyzer_button("exit")
                            .label("Exit")
                            .on_click(|_, window, cx| exit(window, cx)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(panels),
            )
            .child(
                div()
                    .p_3()
                    .border_t_1()
                    .border_color(border)
                    .bg(cx.theme().status_bar)
                    .flex_shrink_0()
                    .child(text(self.status.clone()).text_sm().text_color(muted)),
            )
    }
}

#[cfg(all(test, feature = "ui-test"))]
#[path = "ui_tests.rs"]
mod ui_tests;
