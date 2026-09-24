//! Terminal ▸ Settings… (⌘,): a minimal preferences window with the profile
//! list (the same rows the ⇧⌘P picker shows) and the default font size new
//! windows open with.
//!
//! Both choices persist for windows opened *after* this one closes; an
//! already-open Terminal window keeps its own live profile (⇧⌘P) and font
//! size (⌘+ / ⌘− / ⌘0) — this mirrors "Use Settings as Default" rather than
//! reaching into every other window, which keeps the feature small.

use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, Context, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, WindowHandle,
};
use rmac_ui::{Root, StyledExt as _};

use crate::controller::FONT_SIZE;
use crate::profiles::{self, PROFILES};

const WIDTH: f32 = 320.0;
const HEIGHT: f32 = 420.0;
const MIN_FONT_SIZE: f32 = 8.0;
const MAX_FONT_SIZE: f32 = 32.0;

thread_local! {
    static OPEN: std::cell::Cell<Option<WindowHandle<Root>>> = const { std::cell::Cell::new(None) };
}

/// Terminal ▸ Settings… (⌘,). Opens the one Settings window, or brings the
/// already-open one to the front.
pub(crate) fn show(cx: &mut App) {
    if let Some(handle) = OPEN.with(std::cell::Cell::get) {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    let options = rmac_ui::window_options_for_app(rmac_ui::app_id::TERMINAL, WIDTH, HEIGHT, cx);
    let opened = cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| SettingsView::new(window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    });
    match opened {
        Ok(handle) => OPEN.with(|open| open.set(Some(handle))),
        Err(error) => eprintln!("rmac-terminal: could not open Settings: {error}"),
    }
}

struct SettingsView {
    focus: FocusHandle,
    font_size: f32,
}

impl SettingsView {
    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            font_size: profiles::load_font_size().unwrap_or(FONT_SIZE),
        }
    }

    fn set_default_profile(&mut self, index: usize, cx: &mut Context<Self>) {
        if profiles::save(index).is_ok() {
            cx.notify();
        }
    }

    fn nudge_font(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.font_size = (self.font_size + delta).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        let _ = profiles::save_font_size(self.font_size);
        cx.notify();
    }

    fn toggle_option_as_meta(&mut self, cx: &mut Context<Self>) {
        let enabled = !profiles::load_option_as_meta();
        let _ = profiles::save_option_as_meta(enabled);
        cx.notify();
    }
}

fn section_label(text: &'static str) -> impl IntoElement {
    div()
        .px_3()
        .pt_3()
        .pb_1()
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rmac_ui::mac::text_secondary())
        .child(text)
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let default_profile = profiles::load().map(|(index, _)| index).unwrap_or(1);
        let option_as_meta = profiles::load_option_as_meta();

        div()
            .track_focus(&self.focus)
            .size_full()
            .v_flex()
            .bg(rmac_ui::mac::window())
            .child(rmac_ui::title_bar_content(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rmac_ui::mac::text_secondary())
                    .child("Settings"),
            ))
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(section_label("Profile"))
                    .child(
                        div().px_3().v_flex().child(
                            div()
                                .rounded(px(rmac_ui::mac::radius_control()))
                                .border_1()
                                .border_color(rmac_ui::mac::separator())
                                .overflow_hidden()
                                .children(PROFILES.iter().enumerate().map(|(index, _)| {
                                    let profile = profiles::resolved(index);
                                    let is_default = index == default_profile;
                                    div()
                                        .id(("settings-profile", index))
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .h(px(30.0))
                                        .px_2()
                                        .text_size(px(12.0))
                                        .text_color(rmac_ui::mac::text())
                                        .when(index > 0, |row| {
                                            row.border_t_1().border_color(rmac_ui::mac::separator())
                                        })
                                        .hover(|hovered| hovered.bg(rmac_ui::mac::hover()))
                                        .child(
                                            div()
                                                .w(px(14.0))
                                                .h(px(14.0))
                                                .rounded(px(rmac_ui::mac::radius_menu_item()))
                                                .border_1()
                                                .border_color(rmac_ui::mac::separator())
                                                .bg(gpui::rgb(profile.bg)),
                                        )
                                        .child(div().flex_1().child(profile.name))
                                        .when(is_default, |row| {
                                            row.child(
                                                div().text_color(rmac_ui::mac::accent()).child("✓"),
                                            )
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.set_default_profile(index, cx);
                                        }))
                                })),
                        ),
                    )
                    .child(section_label("Font"))
                    .child(
                        div()
                            .px_3()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                rmac_ui::Button::new("settings-font-smaller", "−")
                                    .small()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.nudge_font(-1.0, cx)),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(40.0))
                                    .text_size(px(12.0))
                                    .text_color(rmac_ui::mac::text())
                                    .child(SharedString::from(format!("{:.0} pt", self.font_size))),
                            )
                            .child(
                                rmac_ui::Button::new("settings-font-bigger", "+")
                                    .small()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.nudge_font(1.0, cx)),
                                    ),
                            ),
                    )
                    .child(section_label("Keyboard"))
                    .child(
                        div()
                            .id("settings-option-as-meta")
                            .px_3()
                            .pb_3()
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_size(px(12.0))
                            .text_color(rmac_ui::mac::text())
                            .child("Use Option as Meta Key")
                            .child(if option_as_meta { "On" } else { "Off" })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_option_as_meta(cx))),
                    ),
            )
    }
}
