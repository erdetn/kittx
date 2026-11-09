use crate::serial_ports::{SerialOptions, discovered_ports};
use crate::ui_theme::apply as apply_recorder_theme;
use crate::vehicle::VehicleOptions;
use crate::{model::*, recording::*};
use gpui_kit::base::Disableable;
use gpui_kit::component::{
    ActiveTheme, Theme, ThemeMode,
    accordion::Accordion,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    combobox::{Combobox, ComboboxEvent, ComboboxState},
    input::{Input, InputEvent, InputState},
    resizable::{h_resizable, resizable_panel},
    scroll::ScrollableElement,
    searchable_list::{SearchableListItem, SearchableVec},
    select::{Select, SelectDelegate, SelectEvent, SelectState},
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use kittx::{
    SerialConfig, TransportConfig,
    datastore::{DiagnosticProfile, MetadataValue, VehicleInfo},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
fn recorder_button(id: impl Into<ElementId>) -> Button {
    Button::new(id).primary().outline().rounded(px(8.))
}

#[derive(Clone)]
struct MeasurementOption {
    id: u16,
    label: String,
}

impl SearchableListItem for MeasurementOption {
    type Value = u16;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }
    fn value(&self) -> &u16 {
        &self.id
    }
}

actions!(recorder, [Start, Stop, Quit]);
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Field {
    Host,
    Port,
    Duration,
    SamplePeriod,
    DtcPeriod,
    Timeout,
    Name,
    ProfileId,
    ProfileVersion,
    Namespace,
    Request,
    Header,
    Filter,
    Target,
}
const FIELDS: [(Field, &str); 14] = [
    (Field::Host, "127.0.0.1"),
    (Field::Port, "36000"),
    (Field::Duration, "300"),
    (Field::SamplePeriod, "1000"),
    (Field::DtcPeriod, "60"),
    (Field::Timeout, "10"),
    (Field::Name, "Trip recording"),
    (Field::ProfileId, "user-dtc"),
    (Field::ProfileVersion, "1"),
    (Field::Namespace, "manufacturer"),
    (Field::Request, ""),
    (Field::Header, ""),
    (Field::Filter, ""),
    (Field::Target, ""),
];
fn text(value: impl Into<SharedString>) -> Div {
    div()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .whitespace_normal()
        .child(value.into())
}
fn shell(id: &'static str, child: impl IntoElement) -> impl IntoElement {
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
pub(crate) struct RecorderApp {
    fields: BTreeMap<Field, Entity<InputState>>,
    settings_sections: BTreeSet<usize>,
    serial_port: Entity<SelectState<SerialOptions>>,
    baud_rate: Entity<SelectState<SerialOptions>>,
    serial_ports_note: Option<String>,
    vehicle_brand: Entity<SelectState<VehicleOptions>>,
    vehicle_model: Entity<SelectState<VehicleOptions>>,
    selected_brand: Option<String>,
    measurements: Entity<ComboboxState<SearchableVec<MeasurementOption>>>,
    dtcs: [bool; 3],
    profile: bool,
    serial: bool,
    timed: bool,
    job: Option<RecordingJob>,
    status: Option<LiveStatus>,
    message: String,
    choosing: bool,
    closing: bool,
    close_after_stop: bool,
    generation: u64,
    focus: FocusHandle,
    window: AnyWindowHandle,
}
impl RecorderApp {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        apply_recorder_theme(cx);
        let fields = FIELDS
            .into_iter()
            .map(|(id, value)| {
                let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
                cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify())
                    .detach();
                (id, input)
            })
            .collect();
        let serial_port = cx.new(|cx| {
            SelectState::new(SerialOptions::new(Vec::new()), None, window, cx).searchable(true)
        });
        cx.subscribe(&serial_port, |_, _, _: &SelectEvent<SerialOptions>, cx| {
            cx.notify()
        })
        .detach();
        let baud_rate = cx.new(|cx| {
            let mut state =
                SelectState::new(SerialOptions::baud_rates(), None, window, cx).searchable(true);
            state.set_selected_value(&"38400".to_owned(), window, cx);
            state
        });
        cx.subscribe(&baud_rate, |_, _, _: &SelectEvent<SerialOptions>, cx| {
            cx.notify()
        })
        .detach();
        // Select provides the kit's searchable, single-choice combobox.
        let vehicle_brand = cx.new(|cx| {
            SelectState::new(VehicleOptions::brands(), None, window, cx).searchable(true)
        });
        let vehicle_model = cx.new(|cx| {
            SelectState::new(VehicleOptions::models(None), None, window, cx).searchable(true)
        });
        cx.subscribe_in(&vehicle_brand, window, |this, _, event, window, cx| {
            let SelectEvent::Confirm(brand) = event;
            let brand = brand.clone();
            if this.selected_brand != brand {
                this.selected_brand = brand;
                this.vehicle_model.update(cx, |state, cx| {
                    state.set_items(
                        VehicleOptions::models(this.selected_brand.as_deref()),
                        window,
                        cx,
                    );
                    state.set_selected_index(None, window, cx);
                });
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(
            &vehicle_model,
            |_, _, _: &SelectEvent<VehicleOptions>, cx| cx.notify(),
        )
        .detach();
        let measurements = cx.new(|cx| {
            let catalog = capture_catalog();
            let defaults = CaptureSettings::default().selected;
            let selected = catalog
                .iter()
                .enumerate()
                .filter(|(_, item)| defaults.contains(&item.id))
                .map(|(index, _)| gpui_kit::component::IndexPath::new(index))
                .collect();
            let options: Vec<_> = catalog
                .into_iter()
                .map(|item| MeasurementOption {
                    id: item.id,
                    label: item.label,
                })
                .collect();
            ComboboxState::new(SearchableVec::new(options), selected, window, cx)
                .multiple(true)
                .searchable(true)
        });
        cx.subscribe(
            &measurements,
            |_, _, _: &ComboboxEvent<SearchableVec<MeasurementOption>>, cx| cx.notify(),
        )
        .detach();
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |app, cx| app.request_close(window, cx))
                .unwrap_or(true)
        });
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            fields,
            settings_sections: [0, 1].into_iter().collect(),
            serial_port,
            baud_rate,
            serial_ports_note: None,
            vehicle_brand,
            vehicle_model,
            selected_brand: None,
            measurements,
            dtcs: [true; 3],
            profile: false,
            serial: false,
            timed: false,
            job: None,
            status: None,
            message: "Choose a connection, select measurements and DTCs, then Start recording."
                .into(),
            choosing: false,
            closing: false,
            close_after_stop: false,
            generation: 0,
            focus,
            window: window.window_handle(),
        }
    }
    fn active(&self) -> bool {
        self.job.as_ref().is_some_and(|j| !j.is_finished())
    }
    fn locked(&self) -> bool {
        self.active() || self.choosing || self.closing
    }
    fn value(&self, field: Field, cx: &App) -> String {
        self.fields[&field].read(cx).value().to_string()
    }
    fn field(&self, field: Field, label: &'static str, _cx: &App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .min_w_0()
            .w_full()
            .flex_shrink_0()
            .child(text(label))
            .child(
                Input::new(&self.fields[&field])
                    .disabled(self.locked())
                    .w_full()
                    .min_w_0(),
            )
    }
    fn choice_field<D: SelectDelegate<Item = String> + 'static>(
        &self,
        state: &Entity<SelectState<D>>,
        id: &'static str,
        label: &'static str,
        placeholder: &'static str,
        disabled: bool,
    ) -> impl IntoElement {
        div()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(text(label))
            .child(
                div()
                    .id(id)
                    .w_full()
                    .min_w_0()
                    .child(
                        Select::new(state)
                            .w_full()
                            .min_w_0()
                            .cleanable(true)
                            .placeholder(placeholder)
                            .accessibility_label(label)
                            .search_placeholder("Search or type a new value…")
                            .menu_max_h(px(240.))
                            .disabled(disabled),
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
    }
    fn refresh_serial_ports(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.serial_port.read(cx).selected_value().cloned();
        let ports = match discovered_ports() {
            Ok(ports) => {
                self.serial_ports_note = ports.is_empty().then(|| {
                    "No serial ports found. You can type a device path or COM name.".into()
                });
                ports
            }
            Err(error) => {
                self.serial_ports_note = Some(format!(
                    "Cannot list serial ports: {error}. You can still enter a port manually."
                ));
                Vec::new()
            }
        };
        let mut options = SerialOptions::new(ports);
        if let Some(value) = &selected {
            options.retain_value(value);
        }
        self.serial_port.update(cx, |state, cx| {
            state.set_items(options, window, cx);
            if let Some(value) = &selected {
                state.set_selected_value(value, window, cx);
            }
        });
        cx.notify();
    }
    fn select_measurements(
        &mut self,
        selected: Vec<u16>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.measurements.update(cx, |state, cx| {
            state.set_selected_values(&selected, window, cx)
        });
        cx.notify();
    }
    fn settings(&self, cx: &App) -> Result<CaptureSettings, String> {
        let transport = if self.serial {
            if !cfg!(feature = "serial") {
                return Err("This build has TCP support only".into());
            }
            TransportConfig::Serial(SerialConfig::new(
                self.serial_port
                    .read(cx)
                    .selected_value()
                    .map(String::as_str)
                    .unwrap_or("")
                    .trim(),
                self.baud_rate
                    .read(cx)
                    .selected_value()
                    .and_then(|rate| rate.trim().parse::<u32>().ok())
                    .filter(|rate| *rate > 0)
                    .ok_or("Baud rate must be a positive integer")?,
            ))
        } else {
            TransportConfig::tcp(
                self.value(Field::Host, cx).trim(),
                self.value(Field::Port, cx)
                    .trim()
                    .parse::<u16>()
                    .map_err(|_| "TCP port must be 1–65535")?,
            )
        };
        let profile = if self.profile {
            let (header, receive_filter, target) = profile_address(
                &self.value(Field::Header, cx),
                &self.value(Field::Filter, cx),
                &self.value(Field::Target, cx),
            )?;
            Some(DiagnosticProfile {
                id: self.value(Field::ProfileId, cx).trim().into(),
                version: self.value(Field::ProfileVersion, cx).trim().into(),
                namespace: self.value(Field::Namespace, cx).trim().into(),
                request: kittx::response::parse_hex(&self.value(Field::Request, cx))
                    .map_err(|e| format!("Manufacturer request: {e}"))?,
                header,
                receive_filter,
                target,
            })
        } else {
            None
        };
        let name = self.value(Field::Name, cx);
        let settings = CaptureSettings {
            producer: "KittxRecorder".into(),
            transport,
            selected: self
                .measurements
                .read(cx)
                .selected_values()
                .into_iter()
                .collect(),
            dtcs: self.dtcs,
            profile,
            duration: if self.timed {
                Some(positive_duration(&self.value(Field::Duration, cx), false)?)
            } else {
                None
            },
            sample_period: positive_duration(&self.value(Field::SamplePeriod, cx), true)?,
            dtc_period: positive_duration(&self.value(Field::DtcPeriod, cx), false)?,
            operation_timeout: positive_duration(&self.value(Field::Timeout, cx), false)?,
            vehicle: VehicleInfo {
                make: self.selected_brand.clone().map(MetadataValue::caller),
                model: self
                    .vehicle_model
                    .read(cx)
                    .selected_value()
                    .cloned()
                    .map(MetadataValue::caller),
                name: (!name.trim().is_empty()).then(|| MetadataValue::caller(name.trim())),
                ..Default::default()
            },
        };
        settings.config()?;
        Ok(settings)
    }
    fn start(&mut self, _: &Start, _: &mut Window, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        let settings = match self.settings(cx) {
            Ok(s) => s,
            Err(e) => {
                self.message = e;
                cx.notify();
                return;
            }
        };
        self.choosing = true;
        self.message =
            "Choose a new .kittx file. No connection is opened until you confirm.".into();
        let directory = std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir());
        let name = format!(
            "trip-{}.kittx",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let picker = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn(async move |this, cx| {
            let result = picker.await;
            let _ = this.update(cx, |this, cx| {
                this.choosing = false;
                if this.closing {
                    return;
                }
                match result {
                    Ok(Ok(Some(path))) => match RecordingJob::spawn(settings, path) {
                        Ok(job) => {
                            this.generation += 1;
                            this.status = Some(job.snapshot());
                            this.job = Some(job);
                            this.message = "Recording worker started".into();
                            this.watch(cx)
                        }
                        Err(e) => this.message = e,
                    },
                    Ok(Ok(None)) => {
                        this.message = "Save cancelled; recording was not started.".into()
                    }
                    error => this.message = format!("Save dialog failed: {error:?}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn stop(&mut self, _: &Stop, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(job) = &self.job {
            job.request_stop();
            self.status = Some(job.snapshot());
            self.message = "Stop requested; waiting for finalization.".into();
            cx.notify();
        }
    }
    fn watch(&self, cx: &mut Context<Self>) {
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        if this.generation != generation {
                            return false;
                        }
                        let Some(job) = &mut this.job else {
                            return false;
                        };
                        this.status = Some(job.snapshot());
                        let finished = job.reap();
                        if finished {
                            this.message = this.status.as_ref().unwrap().message.clone();
                            if this.close_after_stop {
                                let window = this.window;
                                cx.defer(move |cx| {
                                    let _ =
                                        window.update(cx, |_, window, _| window.remove_window());
                                    if cx.windows().is_empty() {
                                        cx.quit();
                                    }
                                });
                            }
                        }
                        cx.notify();
                        !finished
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }
    fn request_close(&mut self, _: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.active() {
            self.close_after_stop = true;
            self.closing = true;
            if let Some(job) = &self.job {
                job.request_stop();
                self.status = Some(job.snapshot());
            }
            self.message = "Closing after the recording is finalized…".into();
            cx.notify();
            false
        } else {
            self.closing = true;
            true
        }
    }
    pub(crate) fn exit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.request_close(window, cx) {
            window.remove_window();
        }
    }
}
impl Render for RecorderApp {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.locked();
        let bg = cx.theme().background;
        let fg = cx.theme().foreground;
        let border = cx.theme().border;
        let mut connection = div()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        recorder_button("tcp")
                            .label(if !self.serial { "● TCP/IP" } else { "TCP/IP" })
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.serial = false;
                                cx.notify()
                            })),
                    )
                    .child(
                        recorder_button("serial")
                            .label(if self.serial { "● Serial" } else { "Serial" })
                            .disabled(busy || !cfg!(feature = "serial"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.serial = true;
                                this.refresh_serial_ports(window, cx);
                            })),
                    ),
            );
        if self.serial {
            connection =
                connection
                    .child(self.choice_field(
                        &self.serial_port,
                        "serial-port",
                        "Serial device / COM port",
                        "Select or type a serial port…",
                        busy,
                    ))
                    .child(
                        recorder_button("refresh-serial-ports")
                            .label("Refresh ports")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.refresh_serial_ports(window, cx)
                            })),
                    )
                    .child(self.choice_field(
                        &self.baud_rate,
                        "baud-rate",
                        "Baud rate (8N1, no flow control)",
                        "Select or type a baud rate…",
                        busy,
                    ));
            if let Some(note) = &self.serial_ports_note {
                connection = connection.child(text(note.clone()).text_sm());
            }
        } else {
            connection = connection
                .child(self.field(Field::Host, "TCP host / IP", cx))
                .child(self.field(Field::Port, "TCP port", cx));
        }
        let mut recording_mode = div()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        recorder_button("manual-mode")
                            .label(if !self.timed {
                                "● Start / stop"
                            } else {
                                "Start / stop"
                            })
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.timed = false;
                                cx.notify()
                            })),
                    )
                    .child(
                        recorder_button("timed-mode")
                            .label(if self.timed {
                                "● Fixed time"
                            } else {
                                "Fixed time"
                            })
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.timed = true;
                                cx.notify()
                            })),
                    ),
            );
        if self.timed {
            recording_mode =
                recording_mode.child(self.field(Field::Duration, "Capture duration (seconds)", cx));
        }
        recording_mode = recording_mode
            .child(self.field(
                Field::SamplePeriod,
                "Nominal measurement period (milliseconds)",
                cx,
            ))
            .child(self.field(Field::DtcPeriod, "DTC scan period (seconds)", cx))
            .child(self.field(Field::Timeout, "Operation timeout (seconds)", cx))
            .child(self.field(Field::Name, "Vehicle / trip name (optional)", cx));
        let vehicle = div().w_full().min_w_0().flex_shrink_0().flex().flex_col().gap_3()
            .child(self.choice_field(
                &self.vehicle_brand, "vehicle-brand", "Car brand", "Select a brand (optional)", busy,
            ))
            .child(self.choice_field(
                &self.vehicle_model, "vehicle-model", "Car model", "Select a model (optional)",
                busy || self.selected_brand.is_none(),
            ))
            .child(text("Choose a brand first. Search or type a value and select its row to confirm. Vehicle details are optional."));
        let accordion = Accordion::new("recorder-settings")
            .multiple(true)
            .w_full()
            .h_auto()
            .min_w_0()
            .flex_shrink_0()
            .item(|item| {
                item.title(text("Connection"))
                    .open(self.settings_sections.contains(&0))
                    .child(connection)
            })
            .item(|item| {
                item.title(text("Recording mode"))
                    .open(self.settings_sections.contains(&1))
                    .child(recording_mode)
            })
            .item(|item| {
                item.title(text("Vehicle type"))
                    .open(self.settings_sections.contains(&2))
                    .child(vehicle)
            })
            .on_toggle_click(cx.listener(|this, indices: &[usize], _, cx| {
                this.settings_sections = indices.iter().copied().collect();
                cx.notify();
            }));
        let mut settings = div()
            .bg(cx.theme().sidebar)
            .id("settings-content")
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(accordion);
        if self.profile {
            settings=settings.child(text("MANUFACTURER DTC READ PROFILE")).child(text("Enter a documented read request. Responses are preserved raw; no vendor interpretation is guessed."));
            for (id, label) in [
                (Field::ProfileId, "Profile ID"),
                (Field::ProfileVersion, "Profile version"),
                (Field::Namespace, "Namespace"),
                (Field::Request, "Read request bytes (hex; required)"),
                (Field::Header, "Header (optional: 3, 6, or 8 hex digits)"),
                (Field::Filter, "Receive filter (optional hex)"),
                (
                    Field::Target,
                    "ECU target (optional: 2, 3, or 8 hex digits)",
                ),
            ] {
                settings = settings.child(self.field(id, label, cx));
            }
        }
        let mut checklist = div()
            .bg(cx.theme().sidebar)
            .id("checklist-content")
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(text("DIAGNOSTIC TROUBLE CODES"));
        for (index, label) in [
            "Stored DTCs (Mode 03, including manufacturer-specific code numbers)",
            "Pending DTCs (Mode 07)",
            "Permanent DTCs (Mode 0A)",
        ]
        .into_iter()
        .enumerate()
        {
            checklist = checklist.child(
                Checkbox::new(("dtc", index))
                    .label(label)
                    .min_w_0()
                    .w_full()
                    .flex_shrink_0()
                    .whitespace_normal()
                    .checked(self.dtcs[index])
                    .disabled(busy)
                    .on_click(cx.listener(move |this, on, _, cx| {
                        this.dtcs[index] = *on;
                        cx.notify()
                    })),
            );
        }
        let measurement_picker = div()
            .id("measurements")
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .child(
                Combobox::new(&self.measurements)
                    .w_full()
                    .min_w_0()
                    .placeholder("Choose measurements…")
                    .search_placeholder("Search measurements or PID…")
                    .menu_max_h(px(320.))
                    .cleanable(true)
                    .disabled(busy),
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
        let measurement_actions = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                recorder_button("decoded")
                    .label("Select decoded")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let values = capture_catalog()
                            .iter()
                            .filter(|item| item.decoded)
                            .map(|item| item.id)
                            .collect();
                        this.select_measurements(values, window, cx);
                    })),
            )
            .child(
                recorder_button("all-signals")
                    .label("Select all PIDs")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let values = capture_catalog().iter().map(|item| item.id).collect();
                        this.select_measurements(values, window, cx);
                    })),
            )
            .child(
                recorder_button("clear-signals")
                    .label("Clear signals")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.select_measurements(Vec::new(), window, cx);
                    })),
            );
        let profile_toggle = Checkbox::new("manufacturer-dtc")
            .label("Manufacturer-specific diagnostic read profile (raw)")
            .min_w_0()
            .w_full()
            .flex_shrink_0()
            .whitespace_normal()
            .checked(self.profile)
            .disabled(busy)
            .on_click(cx.listener(|this, on, _, cx| {
                this.profile = *on;
                cx.notify();
            }));
        checklist = checklist
            .child(profile_toggle)
            .child(text("Selected DTC categories are read at the beginning, periodically, and at the end. Standard reads do not clear DTCs."))
            .child(text("LIVE MEASUREMENTS / MODE 01"))
            .child(measurement_picker)
            .child(measurement_actions)
            .child(text(format!(
                "{} selected · only checked measurements are recorded. Decoded values plot in ViewerUI; other PIDs are stored raw. Selecting all PIDs may include unsupported vehicle data.",
                self.measurements.read(cx).selection().len(),
            )));
        let panels = h_resizable("recorder-panels")
            .child(
                resizable_panel()
                    .size(px(370.))
                    .size_range(px(280.)..px(600.))
                    .child(shell(
                        "settings",
                        settings.overflow_y_scrollbar().id("settings-scroll"),
                    )),
            )
            .child(resizable_panel().child(shell(
                "checklist",
                checklist.overflow_y_scrollbar().id("checklist-scroll"),
            )));
        let mut footer = div()
            .id("recording-status")
            .bg(cx.theme().status_bar)
            .min_w_0()
            .min_h_0()
            .w_full()
            .max_h(px(220.))
            .overflow_y_scroll()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .border_t_1()
            .border_color(border)
            .child(text(self.message.clone()));
        if let Some(status) = &self.status {
            footer = footer
                .child(text(format!(
                    "{:?} · elapsed {:.1}s · acquired {} · committed {} · failed {}",
                    status.phase,
                    status.elapsed.as_secs_f64(),
                    status.progress.acquired,
                    status.progress.committed,
                    status.progress.failed_acquisitions
                )))
                .child(text(format!(
                    "VIN: {} · {}",
                    status.vin.as_deref().unwrap_or("pending / unavailable"),
                    status.path.display()
                )))
                .child(text(status.message.clone()));
            if let Some(error) = &status.last_error {
                footer = footer.child(text(format!("Last error: {error}")));
            }
            if let Some(path) = &status.diagnostics_path {
                footer = footer.child(text(format!("Diagnostic log: {}", path.display())));
            }
        }
        let footer = footer.map(|footer| {
            #[cfg(feature = "ui-test")]
            {
                use gpui_kit::test::TestSupportExt;
                footer.test_support()
            }
            #[cfg(not(feature = "ui-test"))]
            {
                footer
            }
        });
        div()
            .key_context("Recorder")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::start))
            .on_action(cx.listener(Self::stop))
            .on_action(cx.listener(|this, _: &Quit, window, cx| this.exit(window, cx)))
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(bg)
            .text_color(fg)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .p_3()
                    .border_b_1()
                    .border_color(border)
                    .bg(cx.theme().title_bar)
                    .child("KittxRecorder")
                    .child(
                        recorder_button("start")
                            .label(if self.choosing {
                                "Choose file…"
                            } else {
                                "Start recording…"
                            })
                            .disabled(busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.start(&Start, window, cx)),
                            ),
                    )
                    .child(
                        recorder_button("stop")
                            .label("Stop & save")
                            .disabled(!self.active())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.stop(&Stop, window, cx)),
                            ),
                    )
                    .child(
                        recorder_button("theme")
                            .label(if cx.theme().is_dark() {
                                "Light theme"
                            } else {
                                "Dark theme"
                            })
                            .on_click(|_, window, cx| {
                                let mode = if cx.theme().is_dark() {
                                    ThemeMode::Light
                                } else {
                                    ThemeMode::Dark
                                };
                                Theme::change(mode, Some(window), cx);
                                apply_recorder_theme(cx);
                            }),
                    )
                    .child(
                        recorder_button("exit")
                            .label(if self.close_after_stop {
                                "Finalizing before exit…"
                            } else {
                                "Exit"
                            })
                            .on_click(cx.listener(|this, _, window, cx| this.exit(window, cx))),
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
            .child(footer)
    }
}
#[cfg(all(test, feature = "ui-test"))]
#[path = "ui_tests.rs"]
mod ui_tests;
