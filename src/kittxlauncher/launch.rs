#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tool {
    View,
    Record,
    Analyze,
    Probe,
}

impl Tool {
    pub(crate) const ALL: [Self; 4] = [Self::View, Self::Record, Self::Analyze, Self::Probe];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::View => "View",
            Self::Record => "Record",
            Self::Analyze => "Analyze",
            Self::Probe => "Real-Time Probe",
        }
    }

    pub(crate) fn open(self, cx: &mut gpui_kit::App) -> Result<gpui_kit::AnyWindowHandle, String> {
        match self {
            Self::View => kittx_viewer::ui::open(cx, None),
            Self::Record => kittx_recorder::ui::open(cx),
            Self::Analyze => kittx_analyzer::ui::open(cx),
            Self::Probe => kittx_probe::ui::open(cx),
        }
    }

    pub(crate) fn close(
        self,
        handle: gpui_kit::AnyWindowHandle,
        cx: &mut gpui_kit::App,
    ) -> Result<(), String> {
        match self {
            Self::View => kittx_viewer::ui::close(handle, cx),
            Self::Record => kittx_recorder::ui::close(handle, cx),
            Self::Analyze => kittx_analyzer::ui::close(handle, cx),
            Self::Probe => kittx_probe::ui::close(handle, cx),
        }
    }
}
