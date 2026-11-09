use crate::launch::Tool;
use gpui_kit::base::Disableable;
use gpui_kit::component::{
    ActiveTheme,
    button::{Button, ButtonVariants},
};
use gpui_kit::*;

actions!(launcher, [Quit]);

struct Bindings;
impl Global for Bindings {}

pub(crate) fn install_menu(cx: &mut App) {
    if !cx.has_global::<Bindings>() {
        cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
        cx.set_global(Bindings);
    }
    cx.set_menus(vec![
        Menu::new("KittxLauncher").items([MenuItem::action("Quit", Quit)]),
    ]);
}

pub(crate) struct Launcher {
    running: Option<(Tool, AnyWindowHandle)>,
    status: String,
    focus: FocusHandle,
}

impl Launcher {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::ui_theme::apply(cx);
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            weak.update(cx, |app, cx| app.can_close(cx)).unwrap_or(true)
        });
        let parent = window.window_handle();
        let weak = cx.entity().downgrade();
        cx.on_window_closed(move |cx, id| {
            let weak = weak.clone();
            cx.defer(move |cx| {
                let returned = weak
                    .update(cx, |app, cx| {
                        if app
                            .running
                            .is_some_and(|(_, handle)| handle.window_id() == id)
                        {
                            app.running = None;
                            app.status = "Choose an application to open.".into();
                            install_menu(cx);
                            cx.notify();
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if returned {
                    let _ = parent.update(cx, |_, window, cx| {
                        let _ = weak.update(cx, |app, cx| app.focus.focus(window, cx));
                        window.activate_window();
                        cx.activate(true);
                    });
                }
            });
        })
        .detach();
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            running: None,
            status: "Choose an application to open.".into(),
            focus,
        }
    }

    fn open(&mut self, tool: Tool, cx: &mut Context<Self>) {
        if let Some((_, handle)) = self.running {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            return;
        }
        match tool.open(cx) {
            Ok(handle) => {
                self.running = Some((tool, handle));
                self.status = format!("{} is open. Close its window to return here.", tool.label());
                let _ = handle.update(cx, |_, window, _| window.activate_window());
                cx.activate(true);
            }
            Err(error) => {
                self.status = error;
                install_menu(cx);
            }
        }
        cx.notify();
    }

    fn can_close(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some((tool, handle)) = self.running {
            self.status = format!(
                "Closing {} before returning to KittxLauncher…",
                tool.label()
            );
            if let Err(error) = tool.close(handle, cx) {
                self.status = error;
            }
            cx.notify();
            false
        } else {
            true
        }
    }

    pub(crate) fn exit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.can_close(cx) {
            window.remove_window();
            cx.quit();
        }
    }
}

impl Render for Launcher {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Launcher")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Quit, window, cx| this.exit(window, cx)))
            .size_full()
            .p_8()
            .flex()
            .flex_col()
            .gap_4()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().text_2xl().child("KittxLauncher"))
            .children(Tool::ALL.into_iter().map(|tool| {
                Button::new(tool.label())
                    .label(tool.label())
                    .primary()
                    .outline()
                    .w_full()
                    .rounded(px(8.))
                    .disabled(self.running.is_some())
                    .on_click(cx.listener(move |this, _, _, cx| this.open(tool, cx)))
            }))
            .child(
                div()
                    .id("status")
                    .w_full()
                    .whitespace_normal()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.status.clone()),
            )
    }
}

#[cfg(all(test, feature = "ui-test"))]
#[path = "ui_tests.rs"]
mod tests;
