use crate::ui_theme::apply as apply_viewer_theme;
use crate::{document, model};
use document::{Document, Frame};
use gpui_kit::base::Disableable;
use gpui_kit::component::{
    ActiveTheme, IndexPath,
    accordion::Accordion,
    button::{Button, ButtonCustomVariant, ButtonVariants},
    checkbox::Checkbox,
    combobox::{Combobox, ComboboxEvent, ComboboxState},
    resizable::{h_resizable, resizable_panel},
    scroll::ScrollableElement,
    searchable_list::{
        SearchableListChange, SearchableListDelegate, SearchableListItem, SearchableVec,
    },
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use model::{
    MarkerKind, PlotDtcMarker, Viewport, gap, hit_dtc_marker, plot_dtc_markers, range, signal_label,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
fn viewer_button(id: impl Into<ElementId>) -> Button {
    Button::new(id).primary().outline().rounded(px(8.))
}

fn panel_shell(id: &'static str, content: impl IntoElement) -> impl IntoElement {
    div()
        .id(id)
        .size_full()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .child(content)
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

fn wrapped_text(text: impl Into<SharedString>) -> Div {
    div()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .whitespace_normal()
        .child(text.into())
}

fn optional_value(value: Option<impl std::fmt::Display>) -> String {
    value.map_or_else(|| "-".into(), |value| value.to_string())
}

fn metadata_field(label: &str, value: impl std::fmt::Display, cx: &App) -> Div {
    let prefix = format!("{label}: ");
    let text = format!("{prefix}{value}");
    let value_range = prefix.len()..text.len();
    let highlight = (!value_range.is_empty()).then_some((
        value_range,
        HighlightStyle {
            color: Some(cx.theme().foreground),
            font_weight: Some(FontWeight::SEMIBOLD),
            ..Default::default()
        },
    ));
    div()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .whitespace_normal()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(StyledText::new(text).with_highlights(highlight))
}

fn metadata_group<'a>(fields: impl IntoIterator<Item = (&'a str, String)>, cx: &App) -> Div {
    div()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap_1()
        .children(
            fields
                .into_iter()
                .map(|(label, value)| metadata_field(label, value, cx)),
        )
}

actions!(viewer, [Open, Quit]);

/// A single-window application must quit when its native window is closed, too.
pub(crate) fn install_lifecycle(cx: &mut App) {
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}
fn exit_window(window: &mut Window, _: &mut App) {
    window.remove_window();
}

enum Inspection {
    Sample(i64),
    Dtcs(Vec<kittx::datastore::DtcMarker>),
}

#[derive(Clone)]
struct SignalOption {
    index: usize,
    label: String,
    numeric: bool,
}

impl SearchableListItem for SignalOption {
    type Value = usize;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }
    fn value(&self) -> &usize {
        &self.index
    }
    fn disabled(&self) -> bool {
        !self.numeric
    }
}

struct SignalOptions(SearchableVec<SignalOption>);

impl SearchableListDelegate for SignalOptions {
    type Item = SignalOption;

    fn items_count(&self, section: usize) -> usize {
        self.0.items_count(section)
    }
    fn item(&self, index: IndexPath) -> Option<&Self::Item> {
        self.0.item(index)
    }
    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Self::Item: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        self.0.position(value)
    }
    fn perform_search(&mut self, query: &str, window: &mut Window, cx: &mut App) -> Task<()> {
        self.0.perform_search(query, window, cx)
    }
    fn on_will_change(
        &mut self,
        selection: &mut Vec<(IndexPath, Self::Item)>,
        changes: &[SearchableListChange],
    ) {
        // The component disables mouse clicks on raw rows, but keyboard confirm
        // can still propose them. Reject those changes before a check is committed.
        if changes.iter().any(|change| {
            matches!(change, SearchableListChange::Select { index }
                if self.item(*index).is_none_or(|item| !item.numeric))
        }) {
            return;
        }
        self.0.on_will_change(selection, changes);
    }
}

pub(crate) struct Viewer {
    document: Option<Document>,
    frame: Option<Frame>,
    selected: BTreeSet<usize>,
    view: Viewport,
    signals: Entity<ComboboxState<SignalOptions>>,
    signal_catalog_dirty: bool,
    status: String,
    details: String,
    details_scroll: ScrollHandle,
    busy: bool,
    dirty: bool,
    generation: u64,
    bounds: Bounds<Pixels>,
    drag: Option<(Point<Pixels>, Viewport)>,
    box_zoom: bool,
    sidebar: bool,
    dtcs: bool,
    pub(crate) focus: FocusHandle,
    ecu_filter: Option<i64>,
    hidden_categories: BTreeSet<String>,
    show_status: bool,
    hover: Option<i64>,
    pinned: bool,
    inspect_busy: bool,
    pending_inspect: Option<Inspection>,
    inspect_generation: u64,
    open_busy: bool,
    pending_open: Option<PathBuf>,
    dtc_points: Arc<[PlotDtcMarker]>,
    hovered_dtc: Option<i64>,
    selected_dtc: Option<i64>,
    details_visible: bool,
    dtc_mode: kittx::datastore::DtcDisplayMode,
    job_epoch: Arc<AtomicU64>,
    metadata_sections: BTreeSet<usize>,
}
impl Viewer {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        apply_viewer_theme(cx);
        let signals = cx.new(|cx| {
            ComboboxState::new(
                SignalOptions(SearchableVec::new(Vec::new())),
                Vec::new(),
                window,
                cx,
            )
            .multiple(true)
            .searchable(true)
        });
        cx.subscribe(
            &signals,
            |this, _, event: &ComboboxEvent<SignalOptions>, cx| {
                if let ComboboxEvent::Change(values) = event {
                    this.selected = values
                        .iter()
                        .copied()
                        .filter(|&index| {
                            this.document.as_ref().is_some_and(|doc| {
                                doc.overview.series.get(index).is_some_and(|s| s.numeric)
                            })
                        })
                        .collect();
                    this.refresh(cx);
                }
            },
        )
        .detach();
        Self {
            metadata_sections: [0, 1].into_iter().collect(),
            job_epoch: Arc::new(AtomicU64::new(0)),
            dtc_mode: Default::default(),
            dtc_points: Arc::default(),
            hovered_dtc: None,
            selected_dtc: None,
            details_visible: true,
            ecu_filter: None,
            hidden_categories: BTreeSet::new(),
            show_status: false,
            hover: None,
            pinned: false,
            inspect_busy: false,
            pending_inspect: None,
            inspect_generation: 0,
            open_busy: false,
            pending_open: None,
            document: None,
            frame: None,
            selected: BTreeSet::new(),
            view: Viewport::new(0, 1_000_000),
            signals,
            signal_catalog_dirty: true,
            status: "Open a .kittx recording to begin".into(),
            details: String::new(),
            details_scroll: ScrollHandle::default(),
            busy: false,
            dirty: false,
            generation: 0,
            bounds: Bounds::default(),
            drag: None,
            box_zoom: false,
            sidebar: true,
            dtcs: true,
            focus: cx.focus_handle(),
        }
    }
    fn open(&mut self, _: &Open, _: &mut Window, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open .kittx recording".into()),
        });
        cx.spawn(async move |this, cx| match picker.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| this.load(path, cx));
                }
            }
            Ok(Ok(None)) => (),
            error => {
                let _ = this.update(cx, |this, cx| {
                    this.status = format!("File picker failed: {error:?}");
                    cx.notify();
                });
            }
        })
        .detach();
    }
    pub(crate) fn load(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.open_busy {
            self.pending_open = Some(path);
            return;
        }
        self.open_busy = true;
        self.generation += 1;
        let generation = self.generation;
        self.status = "Opening recording…".into();
        let task = cx.background_spawn(async move { Document::open(path) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.open_busy = false;
                if let Some(path) = this.pending_open.take() {
                    this.load(path, cx);
                    return;
                }
                if generation != this.generation {
                    return;
                }
                match result {
                    Ok(doc) => {
                        this.view = doc.full();
                        this.selected = doc
                            .overview
                            .series
                            .iter()
                            .enumerate()
                            .filter(|(_, s)| {
                                s.numeric
                                    && (s.signal == "01:0D"
                                        || s.signal == "01:0C"
                                        || crate::pid::field(&s.signal)
                                            .is_some_and(|f| matches!(f.pid, 0x0c | 0x0d))
                                        || s.signal.contains("speed")
                                        || s.signal.contains("rpm"))
                            })
                            .map(|(i, _)| i)
                            .collect();
                        if this.selected.is_empty()
                            && let Some(i) = doc.overview.series.iter().position(|s| s.numeric)
                        {
                            this.selected.insert(i);
                        }
                        this.ecu_filter = None;
                        this.hover = None;
                        this.pinned = false;
                        this.hovered_dtc = None;
                        this.selected_dtc = None;
                        this.dtc_points = Arc::default();
                        this.details.clear();
                        this.inspect_generation += 1;
                        this.pending_inspect = None;
                        this.document = Some(doc);
                        this.signal_catalog_dirty = true;
                        this.frame = None;
                        this.refresh(cx);
                    }
                    Err(e) => this.status = format!("Cannot open recording: {e}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn sync_signal_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected: Vec<_> = self.selected.iter().copied().collect();
        self.signals.update(cx, |state, cx| {
            state.set_selected_values(&selected, window, cx);
        });
    }

    fn rebuild_signal_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut options: Vec<_> = self
            .document
            .iter()
            .flat_map(|doc| doc.overview.series.iter().enumerate())
            .map(|(index, s)| SignalOption {
                index,
                label: format!(
                    "{} ({}) · {} · ECU {} / S{}{}",
                    signal_label(&s.signal),
                    s.signal,
                    s.unit,
                    s.sender,
                    s.segment_id,
                    if s.numeric { "" } else { " · raw only" }
                ),
                numeric: s.numeric,
            })
            .collect();
        options.sort_by_key(|s| !s.numeric);
        self.signals.update(cx, |state, cx| {
            state.set_items(SignalOptions(SearchableVec::new(options)), window, cx);
        });
        self.sync_signal_selection(window, cx);
        self.signal_catalog_dirty = false;
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let epoch = self.job_epoch.fetch_add(1, Ordering::Relaxed) + 1;
        if self.busy {
            self.dirty = true;
            return;
        }
        let Some(doc) = self.document.clone() else {
            return;
        };
        self.busy = true;
        let generation = self.generation;
        let view = self.view;
        let selected = self.active_selected();
        let pixels = f32::from(self.bounds.size.width).max(1.) as u32;
        let mode = self.dtc_mode;
        let cancellation = self.job_epoch.clone();
        let task = cx.background_spawn(async move {
            doc.frame_with_cancel(view, &selected, pixels, mode, || {
                cancellation.load(Ordering::Relaxed) != epoch
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if generation == this.generation && !this.dirty {
                    match result {
                        Ok(frame) => {
                            this.status = format!(
                                "Read-only · {:.3}–{:.3} s · {} points{}",
                                (view.from - this.document.as_ref().unwrap().overview.origin_us)
                                    as f64
                                    / 1e6,
                                (view.to - this.document.as_ref().unwrap().overview.origin_us)
                                    as f64
                                    / 1e6,
                                frame.samples.iter().map(|(_, p)| p.len()).sum::<usize>(),
                                ""
                            );
                            this.frame = Some(frame)
                        }
                        Err(e) => this.status = e,
                    }
                }
                if this.dirty || generation != this.generation {
                    this.dirty = false;
                    this.refresh(cx)
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn zoom(&mut self, factor: f64, anchor: f64, cx: &mut Context<Self>) {
        self.view = self.view.zoom(factor, anchor);
        self.refresh(cx)
    }
    fn fit(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = &self.document {
            self.view = d.full();
            self.refresh(cx)
        }
    }
    fn active_selected(&self) -> Vec<usize> {
        self.selected
            .iter()
            .copied()
            .filter(|&i| {
                self.ecu_filter.is_none_or(|ecu| {
                    self.document
                        .as_ref()
                        .is_some_and(|d| d.overview.series[i].ecu_id == ecu)
                })
            })
            .collect()
    }
    fn marker_visible(&self, m: &kittx::datastore::DtcMarker) -> bool {
        self.dtcs
            && self.ecu_filter.is_none_or(|e| m.scan.ecu_id == Some(e))
            && !self.hidden_categories.contains(&m.scan.category)
            && (self.show_status
                || m.scan.outcome == "raw_only"
                || m.scan.code_count.is_some_and(|c| c > 0))
    }
    fn inspect(&mut self, at: i64, cx: &mut Context<Self>) {
        self.inspect_target(Inspection::Sample(at), cx)
    }
    fn inspect_target(&mut self, target: Inspection, cx: &mut Context<Self>) {
        if self.pinned {
            self.details_scroll.set_offset(point(px(0.), px(0.)));
        }
        self.inspect_generation += 1;
        if self.inspect_busy {
            self.pending_inspect = Some(target);
            return;
        }
        self.inspect_busy = true;
        let inspection = self.inspect_generation;
        let Some(doc) = self.document.clone() else {
            self.inspect_busy = false;
            return;
        };
        let selected = self.active_selected();
        let generation = self.generation;
        let task = cx.background_spawn(async move {
            match target {
                Inspection::Sample(at) => doc.inspect(at, &selected),
                Inspection::Dtcs(markers) => doc.marker_details(&markers),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.inspect_busy = false;
                if this.generation == generation
                    && this.inspect_generation == inspection
                    && this.pending_inspect.is_none()
                {
                    this.details = result.unwrap_or_else(|e| e);
                    cx.notify()
                }
                if let Some(target) = this.pending_inspect.take() {
                    this.inspect_target(target, cx)
                }
            });
        })
        .detach();
    }
    fn inspect_dtcs(
        &mut self,
        markers: Vec<kittx::datastore::DtcMarker>,
        pin: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(marker) = markers.first() else {
            return;
        };
        self.hover = Some(marker.scan.elapsed_us);
        if pin {
            self.pinned = true;
            self.selected_dtc = Some(marker.scan.id);
            self.details_visible = true;
            self.details = "Loading DTC details…".into();
        }
        self.inspect_target(Inspection::Dtcs(markers), cx);
        cx.notify();
    }
    fn displayed_view(&self) -> Viewport {
        if self.drag.is_some() {
            self.view
        } else {
            self.frame.as_ref().map_or(self.view, |frame| frame.view)
        }
    }
    fn hit_dtc(&self, position: Point<Pixels>) -> Option<PlotDtcMarker> {
        hit_dtc_marker(
            &self.dtc_points,
            f32::from(position.x - self.bounds.left()),
            f32::from(position.y - self.bounds.top()),
            f32::from(self.bounds.size.height),
        )
        .map(|index| self.dtc_points[index].clone())
    }
    fn fraction(&self, p: Point<Pixels>) -> f64 {
        ((p.x - self.bounds.left()) / self.bounds.size.width) as f64
    }
}
impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.signal_catalog_dirty {
            self.rebuild_signal_catalog(window, cx);
        }
        let bg = cx.theme().background;
        let fg = cx.theme().foreground;
        let border = cx.theme().border;
        let unavailable = self.document.is_none() || self.open_busy;
        let signal_picker = div()
            .id("pid-picker")
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .child(
                Combobox::new(&self.signals)
                    .w_full()
                    .min_w_0()
                    .placeholder("Choose PIDs…")
                    .search_placeholder("Search PID, measurement, unit or ECU…")
                    .menu_max_h(px(320.))
                    .disabled(unavailable),
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
            });
        let mut controls = div()
            .id("controls-content")
            .bg(cx.theme().sidebar)
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .overflow_y_scroll()
            .child("RECORDED PIDs")
            .child(signal_picker);
        controls = controls.child(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .child(
                    viewer_button("all")
                        .max_w_full()
                        .min_w_0()
                        .flex_shrink_0()
                        .whitespace_normal()
                        .label("Select all")
                        .disabled(unavailable)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some(d) = &this.document {
                                this.selected = d
                                    .overview
                                    .series
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, s)| s.numeric)
                                    .map(|(i, _)| i)
                                    .collect();
                                this.sync_signal_selection(window, cx);
                                this.refresh(cx)
                            }
                        })),
                )
                .child(
                    viewer_button("clear")
                        .max_w_full()
                        .min_w_0()
                        .flex_shrink_0()
                        .whitespace_normal()
                        .label("Unselect all")
                        .disabled(unavailable)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.selected.clear();
                            this.sync_signal_selection(window, cx);
                            this.refresh(cx)
                        })),
                ),
        );
        controls = controls
            .child(
                Checkbox::new("dtcs")
                    .max_w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .whitespace_normal()
                    .label("DTC observations")
                    .checked(self.dtcs)
                    .on_click(cx.listener(|this, v, _, cx| {
                        this.dtcs = *v;
                        cx.notify()
                    })),
            )
            .child(wrapped_text(
                "DTC = observed read time; fault onset is unknown.",
            ));
        controls = controls.child("ECU filter").child(
            viewer_button("every-ecu")
                .max_w_full()
                .min_w_0()
                .flex_shrink_0()
                .whitespace_normal()
                .label("All ECUs")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ecu_filter = None;
                    this.refresh(cx)
                })),
        );
        if let Some(d) = &self.document {
            let ecus: BTreeSet<_> = d
                .overview
                .series
                .iter()
                .map(|s| (s.ecu_id, s.sender.clone()))
                .collect();
            for (id, sender) in ecus {
                controls = controls.child(
                    viewer_button(("ecu", id as usize))
                        .max_w_full()
                        .min_w_0()
                        .flex_shrink_0()
                        .whitespace_normal()
                        .label(format!(
                            "{} ECU {sender}",
                            if self.ecu_filter == Some(id) {
                                "●"
                            } else {
                                "○"
                            }
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ecu_filter = Some(id);
                            this.refresh(cx)
                        })),
                );
            }
        }
        controls = controls.child(
            Checkbox::new("scan-status")
                .max_w_full()
                .min_w_0()
                .flex_shrink_0()
                .whitespace_normal()
                .label("Include empty / failed scans")
                .checked(self.show_status)
                .on_click(cx.listener(|this, v, _, cx| {
                    this.show_status = *v;
                    cx.notify()
                })),
        );
        if let Some(f) = &self.frame {
            let categories: BTreeSet<_> = f.scans.iter().map(|m| m.scan.category.clone()).collect();
            for category in categories {
                let key = category.clone();
                controls = controls.child(
                    Checkbox::new(SharedString::from(format!("cat-{category}")))
                        .max_w_full()
                        .min_w_0()
                        .flex_shrink_0()
                        .whitespace_normal()
                        .label(category.clone())
                        .checked(!self.hidden_categories.contains(&category))
                        .on_click(cx.listener(move |this, v, _, cx| {
                            if *v {
                                this.hidden_categories.remove(&key);
                            } else {
                                this.hidden_categories.insert(key.clone());
                            }
                            cx.notify()
                        })),
                );
            }
        }
        controls = controls.child(
            viewer_button("dtc-mode")
                .max_w_full()
                .min_w_0()
                .flex_shrink_0()
                .whitespace_normal()
                .label(format!("DTC mode: {:?}", self.dtc_mode))
                .on_click(cx.listener(|this, _, _, cx| {
                    use kittx::datastore::DtcDisplayMode::*;
                    this.dtc_mode = match this.dtc_mode {
                        All => FirstObserved,
                        FirstObserved => SetChanges,
                        SetChanges => All,
                    };
                    this.refresh(cx)
                })),
        );
        let mut info = div()
            .bg(cx.theme().sidebar)
            .id("info-content")
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .p_3()
            .gap_2()
            .overflow_y_scroll()
            .child("RECORDING / VEHICLE");
        if let Some(doc) = &self.document {
            let o = &doc.overview;
            let s = &o.session;
            let mut sections: Vec<Vec<AnyElement>> = (0..5).map(|_| Vec::new()).collect();
            for (index, (section, field)) in [
                (0, metadata_field("File", doc.path.display(), cx)),
                (0, metadata_field("Session", &s.id, cx)),
                (
                    0,
                    metadata_group([("Status", s.status.clone()), ("Schema", "1".into())], cx),
                ),
                (
                    1,
                    metadata_group(
                        [
                            ("VIN", optional_value(s.vin.as_deref())),
                            ("VIN status", s.vin_status.clone()),
                        ],
                        cx,
                    ),
                ),
                (
                    0,
                    metadata_field("Created UTC", format!("{} µs", s.created_utc_us), cx),
                ),
                (
                    0,
                    metadata_group(
                        [
                            ("Started", optional_value(s.started_utc_us)),
                            ("Ended", optional_value(s.ended_utc_us)),
                            ("Finalized", optional_value(s.finalized_utc_us)),
                            ("Exported", optional_value(s.exported_utc_us)),
                        ],
                        cx,
                    ),
                ),
                (
                    0,
                    metadata_field(
                        "Observed",
                        format!("{:.3} seconds", (o.last_us - o.first_us) as f64 / 1e6),
                        cx,
                    ),
                ),
                (
                    0,
                    metadata_field("Cutoff acquisition", s.committed_sequence, cx),
                ),
                (0, metadata_field("Producer", &o.producer, cx)),
                (
                    2,
                    metadata_group(
                        [
                            ("DTC occurrences", o.dtc_occurrences.to_string()),
                            ("Unique codes", o.unique_codes.to_string()),
                        ],
                        cx,
                    ),
                ),
                (
                    2,
                    metadata_group(
                        [
                            ("Manufacturer coverage", s.manufacturer_coverage.clone()),
                            ("Ending scan complete", s.ending_scan_complete.to_string()),
                        ],
                        cx,
                    ),
                ),
            ]
            .into_iter()
            .enumerate()
            {
                sections[section].push(
                    field
                        .id(("metadata", index))
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
                        })
                        .into_any_element(),
                );
            }
            for v in &o.vehicle {
                sections[1].push(
                    metadata_group(
                        [
                            (v.name.as_str(), optional_value(v.value.as_deref())),
                            ("Source", optional_value(v.source.as_deref())),
                            (
                                "Source version",
                                optional_value(v.source_version.as_deref()),
                            ),
                        ],
                        cx,
                    )
                    .into_any_element(),
                );
            }
            for p in &o.policy {
                // Overview summaries separate policy fields with semicolons. Split
                // each field only at its first colon so its value remains intact.
                sections[4].push(
                    metadata_group(
                        p.split("; ").map(|field| {
                            let (label, value) =
                                field.split_once(": ").unwrap_or(("Sampling change", field));
                            (label, value.to_owned())
                        }),
                        cx,
                    )
                    .into_any_element(),
                );
            }
            for c in &o.connections {
                let (label, value) = c.split_once(": ").unwrap_or(("Connection", c.as_str()));
                sections[3].push(metadata_field(label, value, cx).into_any_element());
            }
            let ecus: BTreeSet<_> = o
                .series
                .iter()
                .map(|s| (s.ecu_id, s.segment_id, s.protocol, s.sender.clone()))
                .collect();
            for (id, segment, protocol, sender) in ecus {
                sections[3].push(
                    metadata_group(
                        [
                            ("ECU", id.to_string()),
                            ("Sender", sender),
                            ("Segment", segment.to_string()),
                            ("Protocol", protocol.to_string()),
                        ],
                        cx,
                    )
                    .into_any_element(),
                );
            }
            for s in &o.series {
                sections[4].push(
                    metadata_group(
                        [
                            ("Signal", s.signal.clone()),
                            ("Sender", s.sender.clone()),
                            ("Protocol", s.protocol.to_string()),
                            ("Samples", s.count.to_string()),
                            (
                                "Period",
                                optional_value(s.period_us.map(|period| format!("{period} µs"))),
                            ),
                        ],
                        cx,
                    )
                    .into_any_element(),
                );
            }
            let titles = [
                "Session",
                "Vehicle",
                "Diagnostics",
                "ECUs & connections",
                "Sampling",
            ];
            let mut accordion = Accordion::new("recording-metadata")
                .multiple(true)
                .w_full()
                .h_auto()
                .min_w_0()
                .flex_shrink_0();
            for (index, fields) in sections.into_iter().enumerate() {
                let open = self.metadata_sections.contains(&index);
                accordion = accordion.item(move |item| {
                    item.title(wrapped_text(titles[index])).open(open).child(
                        div()
                            .w_full()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .children(fields),
                    )
                });
            }
            info = info.child(accordion.on_toggle_click(cx.listener(
                |this, indices: &[usize], _, cx| {
                    this.metadata_sections = indices.iter().copied().collect();
                    cx.notify();
                },
            )));
        }
        let toolbar =
            div()
                .min_w_0()
                .w_full()
                .flex_wrap()
                .flex()
                .gap_1()
                .child(
                    viewer_button("open")
                        .label("Open .kittx…")
                        .on_click(cx.listener(|this, _, w, cx| this.open(&Open, w, cx))),
                )
                .child(
                    viewer_button("exit")
                        .label("Exit")
                        .on_click(|_, window, cx| exit_window(window, cx)),
                )
                .child(
                    viewer_button("theme")
                        .label(if cx.theme().is_dark() {
                            "Light theme"
                        } else {
                            "Dark theme"
                        })
                        .on_click(cx.listener(|_, _, window, cx| {
                            use gpui_kit::component::{Theme, ThemeMode};
                            let mode = if cx.theme().is_dark() {
                                ThemeMode::Light
                            } else {
                                ThemeMode::Dark
                            };
                            Theme::change(mode, Some(window), cx);
                            apply_viewer_theme(cx);
                            cx.notify();
                        })),
                )
                .child(viewer_button("side").label("Info").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.sidebar = !this.sidebar;
                        cx.notify()
                    },
                )))
                .child(
                    viewer_button("plus")
                        .label("+")
                        .on_click(cx.listener(|this, _, _, cx| this.zoom(0.5, 0.5, cx))),
                )
                .child(
                    viewer_button("minus")
                        .label("−")
                        .on_click(cx.listener(|this, _, _, cx| this.zoom(2., 0.5, cx))),
                )
                .child(
                    viewer_button("fit")
                        .label("Fit / Auto Y")
                        .on_click(cx.listener(|this, _, _, cx| this.fit(cx))),
                )
                .child(
                    viewer_button("mode")
                        .label(if self.box_zoom { "Box zoom" } else { "Pan" })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.box_zoom = !this.box_zoom;
                            cx.notify()
                        })),
                );
        let mut legend = div()
            .min_w_0()
            .w_full()
            .overflow_hidden()
            .flex()
            .flex_wrap()
            .gap_2();
        if let Some(d) = &self.document {
            for i in self.active_selected() {
                let s = &d.overview.series[i];
                let color = hsla((i as f32 * 0.19) % 1., 0.75, 0.58, 1.);
                legend = legend.child(
                    viewer_button(("legend", i))
                        .max_w_full()
                        .min_w_0()
                        .custom(
                            ButtonCustomVariant::new(cx)
                                .foreground(color)
                                .color(color.opacity(0.1))
                                .hover(color.opacity(0.2))
                                .active(color.opacity(0.3)),
                        )
                        .border_color(color)
                        .label(format!(
                            "● {} [{}] ECU {}",
                            signal_label(&s.signal),
                            s.unit,
                            s.sender
                        ))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.selected.remove(&i);
                            this.sync_signal_selection(window, cx);
                            this.refresh(cx)
                        })),
                );
            }
        }
        if self.dtcs {
            legend = legend.child("◆ DTC read / ○ empty / × failed / □ raw");
        }
        type TrackSeries = (usize, Vec<kittx::datastore::PlotSample>, Option<i64>);
        let mut groups: BTreeMap<String, Vec<TrackSeries>> = BTreeMap::new();
        if let (Some(d), Some(f)) = (&self.document, &self.frame) {
            for (i, points) in &f.samples {
                let s = &d.overview.series[*i];
                groups
                    .entry(s.unit.clone())
                    .or_default()
                    .push((*i, points.clone(), s.period_us));
            }
        }
        let view = self.displayed_view();
        let mut labels = div().flex().flex_wrap().gap_3();
        for (unit, series) in &groups {
            let (lo, hi) = range(
                series
                    .iter()
                    .flat_map(|(_, p, _)| p.iter().map(|p| p.value)),
            );
            labels = labels.child(format!("{unit}: {lo:.2} … {hi:.2}"));
        }
        let mut track_headers = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom(px(24.))
            .flex()
            .flex_col()
            .text_xs();
        for (unit, series) in &groups {
            let (lo, hi) = range(
                series
                    .iter()
                    .flat_map(|(_, p, _)| p.iter().map(|p| p.value)),
            );
            track_headers = track_headers.child(
                div().flex_1().min_h_0().flex().items_start().child(
                    div()
                        .px_1()
                        .bg(bg)
                        .child(format!("{unit} · {lo:.2} … {hi:.2}")),
                ),
            );
        }
        self.dtc_points = plot_dtc_markers(
            self.frame
                .iter()
                .flat_map(|frame| frame.scans.iter())
                .filter(|marker| self.marker_visible(marker)),
            view,
            f32::from(self.bounds.size.width),
        )
        .into();
        let scans = self.dtc_points.clone();
        let hovered_dtc = self.hovered_dtc;
        let selected_dtc = self.selected_dtc;
        let hover = self.hover;
        let weak = cx.entity().downgrade();
        let grid_color = cx.theme().chart_grid.alpha(0.35);
        let plot = canvas(
            move |bounds, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    let changed =
                        (f32::from(this.bounds.size.width) - f32::from(bounds.size.width)).abs()
                            > 32.;
                    this.bounds = bounds;
                    if changed && this.document.is_some() {
                        this.refresh(cx)
                    }
                });
            },
            move |bounds, _, window, _| {
                let width = f32::from(bounds.size.width);
                let height = f32::from(bounds.size.height);
                let track_height = (height - 24.) / groups.len().max(1) as f32;
                let mut grid = PathBuilder::stroke(px(1.));
                for tick in 0..=5 {
                    let x = bounds.left() + px(width * tick as f32 / 5.);
                    grid.move_to(point(x, bounds.top()));
                    grid.line_to(point(x, bounds.bottom()));
                }
                for track in 0..=groups.len().max(1) {
                    let y = bounds.top() + px(track as f32 * track_height);
                    grid.move_to(point(bounds.left(), y));
                    grid.line_to(point(bounds.right(), y));
                }
                if let Ok(path) = grid.build() {
                    window.paint_path(path, grid_color);
                }
                for (track, (_, series)) in groups.into_iter().enumerate() {
                    let (lo, hi) = range(
                        series
                            .iter()
                            .flat_map(|(_, p, _)| p.iter().map(|p| p.value)),
                    );
                    for (i, points, period) in series {
                        let mut runs: Vec<Vec<(f32, f32)>> = Vec::new();
                        let mut previous = None;
                        for p in &points {
                            if !p.value.is_finite() {
                                previous = None;
                                continue;
                            }
                            if previous.is_none_or(|a| gap(a, p, period)) {
                                runs.push(Vec::new());
                            }
                            let x = view.fraction(p.elapsed_us) * width;
                            let y = track as f32 * track_height
                                + 24.
                                + (1. - ((p.value - lo) / (hi - lo)) as f32)
                                    * (track_height - 32.).max(1.);
                            runs.last_mut().unwrap().push((x, y));
                            previous = Some(p);
                        }
                        for run in runs {
                            let single = run.len() == 1;
                            let line = gpui_kit::component::plot::shape::Line::new()
                                .data(run)
                                .x(|p| Some(p.0))
                                .y(|p| Some(p.1))
                                .curve(gpui_kit::component::plot::Curve::Linear)
                                .stroke(hsla((i as f32 * 0.19) % 1., 0.75, 0.58, 1.))
                                .stroke_width(px(1.8));
                            let line = if single {
                                line.dot().dot_size(px(4.))
                            } else {
                                line
                            };
                            line.paint(&bounds, window);
                        }
                    }
                }
                if let Some(at) = hover {
                    let x = bounds.left() + px(view.fraction(at) * width);
                    let mut guide = PathBuilder::stroke(px(1.));
                    guide.move_to(point(x, bounds.top()));
                    guide.line_to(point(x, bounds.bottom()));
                    if let Ok(path) = guide.build() {
                        window.paint_path(path, rgb(0x64748b));
                    }
                }
                for marker in scans.iter() {
                    let kind = marker.kind;
                    let x = bounds.left() + px(marker.x);
                    let y = bounds.bottom() - px(12.);
                    let mut path = PathBuilder::stroke(px(if marker.scans > 1 { 3. } else { 2. }));
                    match kind {
                        MarkerKind::Positive => {
                            path.move_to(point(x, y - px(6.)));
                            path.line_to(point(x + px(5.), y));
                            path.line_to(point(x, y + px(6.)));
                            path.line_to(point(x - px(5.), y));
                            path.line_to(point(x, y - px(6.)));
                        }
                        MarkerKind::Failed => {
                            path.move_to(point(x - px(4.), y - px(4.)));
                            path.line_to(point(x + px(4.), y + px(4.)));
                            path.move_to(point(x - px(4.), y + px(4.)));
                            path.line_to(point(x + px(4.), y - px(4.)));
                        }
                        MarkerKind::Raw => {
                            path.move_to(point(x - px(4.), y - px(4.)));
                            path.line_to(point(x + px(4.), y - px(4.)));
                            path.line_to(point(x + px(4.), y + px(4.)));
                            path.line_to(point(x - px(4.), y + px(4.)));
                            path.line_to(point(x - px(4.), y - px(4.)));
                        }
                        MarkerKind::Empty => {
                            for i in 0..=8 {
                                let a = i as f32 * std::f32::consts::TAU / 8.;
                                let p = point(x + px(a.cos() * 4.), y + px(a.sin() * 4.));
                                if i == 0 {
                                    path.move_to(p)
                                } else {
                                    path.line_to(p)
                                }
                            }
                        }
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(
                            path,
                            match kind {
                                MarkerKind::Positive => rgb(0xf97316),
                                MarkerKind::Failed => rgb(0xef4444),
                                MarkerKind::Raw => rgb(0xa78bfa),
                                MarkerKind::Empty => rgb(0x94a3b8),
                            },
                        );
                    }
                }
                // Paint focus outlines after every point, including overlapping kinds.
                for marker in scans.iter() {
                    let selected = marker
                        .members
                        .iter()
                        .any(|member| Some(member.scan.id) == selected_dtc);
                    if !selected && Some(marker.id) != hovered_dtc {
                        continue;
                    }
                    let x = bounds.left() + px(marker.x);
                    let y = bounds.bottom() - px(12.);
                    let mut outline = PathBuilder::stroke(px(if selected { 2.5 } else { 1.5 }));
                    outline.move_to(point(x - px(8.), y - px(9.)));
                    outline.line_to(point(x + px(8.), y - px(9.)));
                    outline.line_to(point(x + px(8.), y + px(9.)));
                    outline.line_to(point(x - px(8.), y + px(9.)));
                    outline.line_to(point(x - px(8.), y - px(9.)));
                    if let Ok(path) = outline.build() {
                        window.paint_path(path, fg);
                    }
                }
            },
        )
        .size_full();
        let plot = div()
            .id("plot")
            .map(|plot| {
                #[cfg(feature = "ui-test")]
                {
                    use gpui_kit::test::TestSupportExt;
                    plot.test_support()
                }
                #[cfg(not(feature = "ui-test"))]
                {
                    plot
                }
            })
            .relative()
            .flex_1()
            .min_h(px(180.))
            .overflow_hidden()
            .when(self.hovered_dtc.is_some(), |plot| {
                plot.cursor(CursorStyle::PointingHand)
            })
            .child(plot)
            .child(track_headers)
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, cx| {
                let delta = e.delta.pixel_delta(px(20.));
                this.zoom(
                    if delta.y > px(0.) { 0.85 } else { 1.18 },
                    this.fraction(e.position),
                    cx,
                )
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    if let Some(marker) = this.hit_dtc(e.position) {
                        this.drag = None;
                        this.inspect_dtcs(marker.members, true, cx);
                        cx.stop_propagation();
                        return;
                    }
                    this.drag = Some((e.position, this.view));
                    cx.notify()
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if let Some((start, original)) = this.drag {
                    if e.pressed_button == Some(MouseButton::Left) && !this.box_zoom {
                        this.view = original
                            .pan(((start.x - e.position.x) / this.bounds.size.width) as f64);
                        cx.notify()
                    }
                } else {
                    let marker = this.hit_dtc(e.position);
                    let previous = this.hovered_dtc;
                    this.hovered_dtc = marker.as_ref().map(|marker| marker.id);
                    if !this.pinned {
                        if let Some(marker) = marker {
                            if previous != Some(marker.id) {
                                this.inspect_dtcs(marker.members, false, cx);
                            }
                        } else {
                            let at = this.displayed_view().time(this.fraction(e.position));
                            this.hover = Some(at);
                            this.inspect(at, cx);
                        }
                    }
                    if previous != this.hovered_dtc || !this.pinned {
                        cx.notify();
                    }
                }
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !*hovered {
                    this.hovered_dtc = None;
                    cx.notify();
                }
            }))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.drag.take().is_some() {
                        this.refresh(cx);
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, e: &MouseUpEvent, _, cx| {
                    if let Some((start, original)) = this.drag.take() {
                        let movement = f32::from(e.position.x - start.x).abs();
                        if movement < 4. {
                            this.pinned = true;
                            this.selected_dtc = None;
                            let at = this.displayed_view().time(this.fraction(e.position));
                            this.hover = Some(at);
                            this.inspect(at, cx)
                        } else {
                            if this.box_zoom {
                                let a = original.time(this.fraction(start));
                                let b = original.time(this.fraction(e.position));
                                this.view = Viewport::new(a.min(b), a.max(b));
                            }
                            this.refresh(cx)
                        }
                    }
                }),
            );
        let message = if self.busy {
            "Loading viewport…"
        } else if self
            .document
            .as_ref()
            .is_some_and(|d| d.overview.series.is_empty() && d.overview.last_us == 0)
        {
            "This recording has no committed observations."
        } else if self
            .document
            .as_ref()
            .is_some_and(|d| !d.overview.series.iter().any(|s| s.numeric))
        {
            "No numeric samples; raw signals and DTC history are available."
        } else if self.document.is_none() {
            "Open a recording using File → Open or Ctrl/Cmd+O."
        } else if self.selected.is_empty() {
            "No numeric signals selected. DTC history remains visible."
        } else {
            "Wheel: zoom · Drag: pan/box zoom · Click: original sample or DTC details"
        };
        let mut history = div()
            .id("dtc-history")
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_1()
            .overflow_y_scroll()
            .max_h(px(120.));
        if let Some(f) = &self.frame {
            for (i, marker) in f
                .scans
                .iter()
                .enumerate()
                .filter(|(_, m)| self.ecu_filter.is_none_or(|e| m.scan.ecu_id == Some(e)))
                .take(200)
            {
                let at = marker.scan.elapsed_us;
                let marker = marker.clone();
                let label = format!(
                    "{:.3}s · {} / {} · ECU {} · {} · {} codes · {} scans",
                    (at - self.document.as_ref().unwrap().overview.origin_us) as f64 / 1e6,
                    marker.scan.category,
                    marker.scan.namespace,
                    optional_value(marker.scan.ecu_id),
                    marker.scan.outcome,
                    optional_value(marker.scan.code_count),
                    marker.scans
                );
                history = history.child(
                    viewer_button(("history", i))
                        .w_full()
                        .min_w_0()
                        .flex_shrink_0()
                        .tooltip(label.clone())
                        .label(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.inspect_dtcs(vec![marker.clone()], true, cx)
                        })),
                );
            }
        }
        let chart = div()
            .id("chart")
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
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .size_full()
            .gap_2()
            .p_3()
            .child(legend)
            .child(labels)
            .child(wrapped_text(message))
            .child(
                viewer_button("details-toggle")
                    .label(if self.details_visible {
                        "Hide details"
                    } else {
                        "Show details"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.details_visible = !this.details_visible;
                        cx.notify()
                    })),
            )
            .child(plot)
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .justify_between()
                    .gap_2()
                    .child(format!(
                        "{:.3} s",
                        (view.from - self.document.as_ref().map_or(0, |d| d.overview.origin_us))
                            as f64
                            / 1e6
                    ))
                    .child(format!(
                        "{:.3} s",
                        (view.to - self.document.as_ref().map_or(0, |d| d.overview.origin_us))
                            as f64
                            / 1e6
                    )),
            )
            .child(
                div()
                    .id("details")
                    .bg(cx.theme().sidebar)
                    .map(|details| {
                        #[cfg(feature = "ui-test")]
                        {
                            use gpui_kit::test::TestSupportExt;
                            details.test_support()
                        }
                        #[cfg(not(feature = "ui-test"))]
                        {
                            details
                        }
                    })
                    .w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .overflow_hidden()
                    .h(px(if self.details_visible { 220. } else { 0. }))
                    .relative()
                    .overflow_y_scroll()
                    .track_scroll(&self.details_scroll)
                    .p_2()
                    .border_1()
                    .border_color(border)
                    .child(wrapped_text(self.details.clone()))
                    .child(history.overflow_y_scrollbar().id("history-scroll"))
                    .vertical_scrollbar(&self.details_scroll),
            );
        let mut panels = h_resizable("main-panels");
        if self.sidebar {
            panels = panels.child(
                resizable_panel()
                    .size(px(280.))
                    .size_range(px(180.)..px(500.))
                    .child(panel_shell(
                        "info",
                        info.overflow_y_scrollbar().id("info-scroll"),
                    )),
            )
        }
        panels = panels
            .child(
                resizable_panel()
                    .size(px(260.))
                    .size_range(px(180.)..px(500.))
                    .child(panel_shell(
                        "controls",
                        controls.overflow_y_scrollbar().id("controls-scroll"),
                    )),
            )
            .child(resizable_panel().child(chart));
        div()
            .key_context("Viewer")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::open))
            .on_action(|_: &Quit, window, cx| exit_window(window, cx))
            .on_key_down(cx.listener(
                |this, e: &KeyDownEvent, _, cx| match e.keystroke.key.as_str() {
                    "+" | "=" => this.zoom(0.5, 0.5, cx),
                    "-" => this.zoom(2., 0.5, cx),
                    "f" => this.fit(cx),
                    "left" => {
                        this.view = this.view.pan(-0.2);
                        this.refresh(cx)
                    }
                    "right" => {
                        this.view = this.view.pan(0.2);
                        this.refresh(cx)
                    }
                    "escape" => {
                        this.drag = None;
                        this.pinned = false;
                        this.hover = None;
                        this.hovered_dtc = None;
                        this.selected_dtc = None;
                        this.pending_inspect = None;
                        this.inspect_generation += 1;
                        this.details.clear();
                        cx.notify()
                    }
                    _ => (),
                },
            ))
            .size_full()
            .flex()
            .flex_col()
            .bg(bg)
            .text_color(fg)
            .child(
                div()
                    .p_2()
                    .bg(cx.theme().title_bar)
                    .border_b_1()
                    .border_color(border)
                    .child(toolbar),
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
                    .p_2()
                    .border_t_1()
                    .border_color(border)
                    .bg(cx.theme().status_bar)
                    .w_full()
                    .min_w_0()
                    .overflow_hidden()
                    .child(wrapped_text(self.status.clone())),
            )
    }
}

#[cfg(all(test, feature = "ui-test"))]
#[path = "ui_tests.rs"]
mod ui_tests;
