//! Application ▸ Settings… (⌘,, PRV-MENU-001 / PRV-SETTINGS-001): app-wide
//! preferences, persisted to `~/.config/rmac/preview.json`
//! (`rmac_preview::settings_store`). Preview has no single "main" window to
//! bind this to (it is a multi-window, multi-document app), so unlike
//! Notes' Settings this one is not tied to any document: it edits defaults
//! that take effect for windows opened *after* a change, the same way a
//! change to many of macOS's own Settings panes does not retroactively
//! rewrite windows already open.
//!
//! Three tabs, matching the toolbar measured on the real Mac
//! (`tests/inventory/mac/Preview.json`'s `settings.controls`): General
//! (the one pane fully captured there — a "Window background:" colour well
//! and a Reset button), Images and PDF. The Mac's own Images/PDF panes were
//! not captured by that run, so their content here is Preview's own
//! existing, genuinely-working per-window toggles (Show Image Background,
//! Use Dark Appearance for PDF) exposed as the *default* for new windows,
//! rather than inventing unmeasured controls.

use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, Context, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, WindowHandle,
};
use rmac_preview::settings_store::{self, Settings};
use rmac_ui::{Button, Checkbox, Root, StyledExt as _};

use crate::ShowSettings;

const WIDTH: f32 = 456.0;
const HEIGHT: f32 = 300.0;

/// Settings ▸ General ▸ Window background presets. The real Mac's single
/// colour well opens a full OS colour panel; without one, Preview offers
/// the measured default plus a short, honest set of neutral presets
/// instead of pretending to be a full picker.
const BACKGROUND_PRESETS: [(&str, u32); 5] = [
    ("white", 0xFFFFFF),
    ("default", 0xE9E9ED),
    ("medium-grey", 0xC7C7CC),
    ("dark-grey", 0x3A3A3C),
    ("black", 0x000000),
];

thread_local! {
    static OPEN: std::cell::Cell<Option<WindowHandle<Root>>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn show(cx: &mut App) {
    if let Some(handle) = OPEN.with(std::cell::Cell::get) {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    // Not `window_options_for_app`: Preview has no single main window, but
    // any document window opened with the same `app_id::PREVIEW` identity
    // could otherwise still collide with Settings' own persisted geometry
    // key (the UIA-06/UIA-09 window-geometry-key bug).
    let options = rmac_ui::window_options_for_panel(rmac_ui::app_id::PREVIEW, WIDTH, HEIGHT, cx);
    match cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        window.set_window_title("Preview Settings");
        let view = cx.new(|cx| SettingsView::new(window, cx));
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            OPEN.with(|open| open.set(Some(handle)));
        }
        Err(error) => eprintln!("rmac-preview: could not open Settings: {error}"),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    Images,
    Pdf,
}

struct SettingsView {
    focus: FocusHandle,
    tab: Tab,
    settings: Settings,
}

impl SettingsView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // `register_menu_target` below already tracks this as a key-window
        // candidate; `observe_window_state` would additionally persist its
        // geometry under `app_id::PREVIEW` (the UIA-06/UIA-09 bug).
        let focus = cx.focus_handle();
        rmac_ui::register_menu_target(window, &focus, cx);
        Self {
            focus,
            tab: Tab::General,
            settings: settings_store::load_settings().unwrap_or_default(),
        }
    }

    fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        cx.notify();
    }

    fn set_background(&mut self, color: u32, cx: &mut Context<Self>) {
        self.settings.window_background = color;
        self.save(cx);
    }

    fn reset_background(&mut self, cx: &mut Context<Self>) {
        self.settings.window_background = Settings::default().window_background;
        self.save(cx);
    }

    fn set_show_image_background_default(&mut self, value: bool, cx: &mut Context<Self>) {
        self.settings.show_image_background_default = value;
        self.save(cx);
    }

    fn set_dark_appearance_for_pdf_default(&mut self, value: bool, cx: &mut Context<Self>) {
        self.settings.dark_appearance_for_pdf_default = value;
        self.save(cx);
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings.clone();
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = settings_store::save_settings(&settings) {
                    eprintln!("rmac-preview: could not save Settings: {error}");
                }
            })
            .detach();
        cx.notify();
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.tab;
        let background = self.settings.window_background;
        let show_image_background_default = self.settings.show_image_background_default;
        let dark_appearance_for_pdf_default = self.settings.dark_appearance_for_pdf_default;
        div()
            .track_focus(&self.focus)
            .key_context("Preview")
            .on_action(cx.listener(|_, _: &rmac_ui::RequestClose, window, _| {
                window.remove_window();
            }))
            .on_action(cx.listener(|_, _: &ShowSettings, window, _| window.activate_window()))
            .size_full()
            .v_flex()
            .bg(rmac_ui::mac::window())
            .text_color(rmac_ui::mac::text())
            .child(rmac_ui::title_bar_content(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_weight(FontWeight::BOLD)
                    .child("Preview Settings"),
            ))
            .child(
                div().flex().gap_2().px_4().py_2().children(
                    [
                        ("General", Tab::General),
                        ("Images", Tab::Images),
                        ("PDF", Tab::Pdf),
                    ]
                    .into_iter()
                    .map(|(label, value)| {
                        Button::new(format!("preview-settings-tab-{label}"), label)
                            .small()
                            .selected(tab == value)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_tab(value, cx);
                            }))
                    }),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .v_flex()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .when(tab == Tab::General, |pane| {
                        pane.child("Window background:").child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .children(BACKGROUND_PRESETS.into_iter().map(|(name, color)| {
                                    div()
                                        .id(gpui::SharedString::from(format!(
                                            "preview-settings-background-{name}"
                                        )))
                                        .size(px(22.0))
                                        .rounded_full()
                                        .bg(gpui::rgb(color))
                                        .border_2()
                                        .border_color(if background == color {
                                            rmac_ui::mac::accent()
                                        } else {
                                            gpui::transparent_black()
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.set_background(color, cx);
                                        }))
                                }))
                                .child(
                                    Button::new("preview-settings-background-reset", "Reset")
                                        .small()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.reset_background(cx);
                                        })),
                                ),
                        )
                    })
                    .when(tab == Tab::Images, |pane| {
                        pane.child(
                            Checkbox::new("preview-settings-show-image-background")
                                .label("Show image background in new windows")
                                .checked(show_image_background_default)
                                .on_change(cx.listener(|this, value: &bool, _, cx| {
                                    this.set_show_image_background_default(*value, cx);
                                })),
                        )
                    })
                    .when(tab == Tab::Pdf, |pane| {
                        pane.child(
                            Checkbox::new("preview-settings-dark-appearance-for-pdf")
                                .label("Use dark appearance for PDF in new windows")
                                .checked(dark_appearance_for_pdf_default)
                                .on_change(cx.listener(|this, value: &bool, _, cx| {
                                    this.set_dark_appearance_for_pdf_default(*value, cx);
                                })),
                        )
                    }),
            )
    }
}
