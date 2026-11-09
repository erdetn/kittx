//! Native live plots reuse the viewer's units, ranges and continuity rules.
use crate::app::ProbeApp;
use crate::live::{LiveSnapshot, SignalPlot, UnitTrack};
use gpui_kit::base::Disableable;
use gpui_kit::component::{
    ActiveTheme,
    button::{Button, ButtonVariants},
    scroll::ScrollableElement,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use kittx_viewer::model::{Viewport, gap, range};
use std::{cell::Cell, rc::Rc};

fn color(signal: &SignalPlot) -> Hsla {
    let identity = format!("{}:{}:{}", signal.signal, signal.sender, signal.segment);
    let hash = identity.bytes().fold(2166136261u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16777619)
    });
    hsla((hash % 360) as f32 / 360., 0.75, 0.58, 1.)
}

fn track(
    index: usize,
    track: &UnitTrack,
    snapshot: &LiveSnapshot,
    view: Viewport,
    cx: &mut Context<ProbeApp>,
) -> impl IntoElement {
    let (lo, hi) = range(track.signals.iter().flat_map(|s| {
        s.points
            .iter()
            .filter(|p| p.elapsed_us >= view.from && p.elapsed_us <= view.to)
            .map(|p| p.value)
    }));
    let curves = track.signals.clone();
    let mut legend = div()
        .id(("track-legend", index))
        .w_full()
        .min_w_0()
        .max_h(px(70.))
        .overflow_y_scroll()
        .flex_shrink_0()
        .flex()
        .flex_wrap()
        .gap_2();
    for signal in &track.signals {
        let value = signal
            .points
            .iter()
            .rev()
            .find(|p| p.value.is_finite() && p.elapsed_us >= view.from && p.elapsed_us <= view.to)
            .map_or_else(
                || "no samples in this interval".into(),
                |p| format!("{:.2} {}", p.value, track.unit),
            );
        legend = legend.child(div().text_xs().text_color(color(signal)).child(format!(
            "● {} · ECU {} / S{} · {value}",
            signal.label, signal.sender, signal.segment,
        )));
    }
    let bounds = Rc::new(Cell::new(Bounds::<Pixels>::default()));
    let painted_bounds = bounds.clone();
    let wheel_bounds = bounds.clone();
    let grid_color = cx.theme().chart_grid.alpha(0.35);
    let plot = canvas(
        move |bounds, _, _| painted_bounds.set(bounds),
        move |bounds, _, window, _| {
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let mut grid = PathBuilder::stroke(px(1.));
            for tick in 0..=4 {
                let y = bounds.top() + px(height * tick as f32 / 4.);
                grid.move_to(point(bounds.left(), y));
                grid.line_to(point(bounds.right(), y));
            }
            for tick in 0..=5 {
                let x = bounds.left() + px(width * tick as f32 / 5.);
                grid.move_to(point(x, bounds.top()));
                grid.line_to(point(x, bounds.bottom()));
            }
            if let Ok(grid) = grid.build() {
                window.paint_path(grid, grid_color);
            }
            for signal in &curves {
                let mut runs = Vec::<Vec<(f32, f32)>>::new();
                let mut previous = None;
                for p in &signal.points {
                    if !p.value.is_finite() {
                        previous = None;
                        continue;
                    }
                    if previous.is_none_or(|last| gap(last, p, None)) {
                        runs.push(Vec::new());
                    }
                    let x = view.fraction(p.elapsed_us) * width;
                    let y =
                        8. + (1. - ((p.value - lo) / (hi - lo)) as f32) * (height - 16.).max(1.);
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
                        .stroke(color(signal))
                        .stroke_width(px(1.8));
                    let line = if single {
                        line.dot().dot_size(px(4.))
                    } else {
                        line
                    };
                    line.paint(&bounds, window);
                }
            }
        },
    )
    .size_full();
    div()
        .id(("unit-track", index))
        .bg(cx.theme().sidebar)
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(px(8.))
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
        .child(
            div()
                .text_sm()
                .child(format!("{} · Y {:.2} … {:.2}", track.unit, lo, hi)),
        )
        .child(legend.overflow_y_scrollbar().id(("unit-legend", index)))
        .child(
            div()
                .id(("unit-plot", index))
                .bg(cx.theme().background)
                .h(px(160.))
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .overflow_hidden()
                .cursor(CursorStyle::OpenHand)
                .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _, cx| {
                    let bounds = wheel_bounds.get();
                    let delta = event.delta.pixel_delta(px(20.));
                    if delta.y != px(0.) && bounds.size.width > px(0.) {
                        let anchor =
                            f64::from((event.position.x - bounds.left()) / bounds.size.width);
                        this.zoom_plot(if delta.y > px(0.) { 0.85 } else { 1. / 0.85 }, anchor, cx);
                        cx.stop_propagation();
                    }
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.begin_plot_pan(event.position, f32::from(bounds.get().size.width));
                        cx.stop_propagation();
                    }),
                )
                .on_mouse_move(
                    cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        this.move_plot_pan(event, cx)
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.end_plot_pan()),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.end_plot_pan()),
                )
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
                .child(plot),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .text_xs()
                .child(format!(
                    "{:.3} s",
                    (view.from - snapshot.origin_us) as f64 / 1e6
                ))
                .child(format!(
                    "{:.3} s",
                    (view.to - snapshot.origin_us) as f64 / 1e6
                )),
        )
}

pub(crate) fn render(
    snapshot: Option<&LiveSnapshot>,
    message: &str,
    view: Option<Viewport>,
    active: bool,
    cx: &mut Context<ProbeApp>,
) -> Div {
    let button = |id, label| {
        Button::new(id)
            .label(label)
            .primary()
            .outline()
            .rounded(px(8.))
            .disabled(snapshot.is_none())
    };
    let controls = div()
        .id("plot-controls")
        .w_full()
        .flex_shrink_0()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .child(
            button("plot-zoom-in", "Zoom +")
                .on_click(cx.listener(|this, _, _, cx| this.zoom_plot(0.5, 0.5, cx))),
        )
        .child(
            button("plot-zoom-out", "Zoom −")
                .on_click(cx.listener(|this, _, _, cx| this.zoom_plot(2., 0.5, cx))),
        )
        .child(
            button("plot-earlier", "← Earlier")
                .on_click(cx.listener(|this, _, _, cx| this.pan_plot(-0.25, cx))),
        )
        .child(
            button("plot-later", "Later →")
                .on_click(cx.listener(|this, _, _, cx| this.pan_plot(0.25, cx))),
        )
        .child(
            button(
                "plot-reset",
                if active { "Follow live" } else { "Fit session" },
            )
            .on_click(cx.listener(|this, _, _, cx| this.reset_plot(cx))),
        );
    let mut plots = div()
        .id("probe-plots-content")
        .w_full()
        .min_w_0()
        .min_h_0()
        .flex_1()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_3();
    let mut dtcs = div()
        .id("probe-dtcs-content")
        .bg(cx.theme().sidebar)
        .w_full()
        .min_w_0()
        .h_full()
        .flex_shrink_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_2()
        .p_2()
        .border_1()
        .border_color(cx.theme().border)
        .child("DTC READS · newest first · read time is not fault onset");
    if let Some(snapshot) = snapshot {
        for (index, unit) in snapshot.tracks.iter().enumerate() {
            plots = plots.child(track(
                index,
                unit,
                snapshot,
                view.unwrap_or(snapshot.view),
                cx,
            ));
        }
        if snapshot.tracks.is_empty() {
            plots = plots.child("No numeric samples yet. Selected status/raw-only PIDs are retained in the capture.");
        }
        if snapshot.raw_series > 0 {
            plots = plots.child(format!(
                "Original raw evidence retained for {} source series.",
                snapshot.raw_series
            ));
        }
        if snapshot.dtc_history_limited {
            dtcs = dtcs.child("Latest 200 reads shown; the recording retains the full history.");
        }
        for read in &snapshot.dtcs {
            let codes = if !read.codes.is_empty() {
                read.codes.join(", ")
            } else if read.scan.outcome == "ok" && read.scan.code_count == Some(0) {
                "No DTCs returned".into()
            } else if let Some(raw) = &read.scan.raw_payload {
                format!("Raw response · {} bytes", raw.len())
            } else {
                "No decoded code result".into()
            };
            dtcs = dtcs.child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .whitespace_normal()
                    .text_sm()
                    .child(format!(
                        "{:.3}s · {} · ECU {} · {} · {}{}",
                        (read.scan.elapsed_us - snapshot.origin_us) as f64 / 1e6,
                        read.scan.category,
                        read.sender,
                        read.scan.outcome,
                        codes,
                        read.scan
                            .error
                            .as_ref()
                            .map_or_else(String::new, |e| format!(" · {e}"))
                    )),
            );
        }
        if snapshot.dtcs.is_empty() {
            dtcs = dtcs.child("No selected DTC read has completed yet.");
        }
    } else {
        plots = plots.child("Select PIDs and DTC types in the side panel, then Start.");
    }
    let plots = div()
        .id("probe-plots")
        .w_full()
        .min_w_0()
        .min_h_0()
        .flex_1()
        .child(plots.overflow_y_scrollbar().id("probe-plots-scroll"))
        .map(|plots| {
            #[cfg(feature = "ui-test")]
            {
                use gpui_kit::test::TestSupportExt;
                plots.test_support()
            }
            #[cfg(not(feature = "ui-test"))]
            {
                plots
            }
        });
    let dtcs = div()
        .id("probe-dtcs")
        .w_full()
        .min_w_0()
        .h(px(210.))
        .flex_shrink_0()
        .child(dtcs.overflow_y_scrollbar().id("probe-dtcs-scroll"))
        .map(|dtcs| {
            #[cfg(feature = "ui-test")]
            {
                use gpui_kit::test::TestSupportExt;
                dtcs.test_support()
            }
            #[cfg(not(feature = "ui-test"))]
            {
                dtcs
            }
        });
    div()
        .size_full()
        .min_w_0()
        .min_h_0()
        .flex()
        .flex_col()
        .gap_3()
        .p_3()
        .child(
            div()
                .flex_shrink_0()
                .whitespace_normal()
                .child(message.to_string()),
        )
        .child(controls)
        .child(div().flex_shrink_0().text_xs().whitespace_normal().child("Grouped by unit · shared time axis · Wheel over a plot: zoom · Drag: pan · Scroll between plots: more units"))
        .child(plots)
        .child(dtcs)
}
