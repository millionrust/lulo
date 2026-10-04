//! Terminal ▸ Settings… (⌘,): Profile, Font, Text (cursor style/blink),
//! Window (size), Shell (when it exits), General (new-window working
//! directory) and Keyboard, as one scrolling page — mocked in
//! `design-lab/terminal-settings.html`. TERM-17 tracks the Mac's own
//! 667×628 tabbed Profiles/General window; this keeps the single-page
//! layout the app already shipped and just adds sections to it.
//!
//! Every choice here persists for windows/tabs opened *after* this one
//! closes; an already-open Terminal window keeps its own live profile
//! (⇧⌘P), font size (⌘+ / ⌘− / ⌘0) and cursor — this mirrors "Use Settings
//! as Default" rather than reaching into every other window, which keeps
//! the feature small. The one exception is Shell ▸ "when the shell exits",
//! which is re-read from disk the moment any open window's shell actually
//! exits (`controller/tab_lifecycle.rs`), so it does apply to windows
//! already open.

use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, Context, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, WindowHandle,
};
use rmac_ui::{Checkbox, Root, StyledExt as _};

use crate::controller::FONT_SIZE;
use crate::profiles::{self, PROFILES};
use crate::settings::{
    self, AskBeforeClosing, CursorStyle, NewWindowWorkingDirectory, ShellExitBehavior,
};

const WIDTH: f32 = 320.0;
const HEIGHT: f32 = 620.0;
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
        Ok(handle) => {
            OPEN.with(|open| open.set(Some(handle)));
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
        Err(error) => eprintln!("rmac-terminal: could not open Settings: {error}"),
    }
}

struct SettingsView {
    focus: FocusHandle,
    font_size: f32,
    /// The non-profile, non-font values this window edits, kept in one
    /// place so a single failed save shows one message instead of several.
    settings: settings::Settings,
    save_error: Option<SharedString>,
}

impl SettingsView {
    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (settings, save_error) = match settings::load() {
            Ok(settings) => (settings, None),
            Err(failure) => (
                settings::Settings::default(),
                Some(SharedString::from(failure.to_string())),
            ),
        };
        Self {
            focus: cx.focus_handle(),
            font_size: profiles::load_font_size().unwrap_or(FONT_SIZE),
            settings,
            save_error,
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

    fn save_settings(&mut self, cx: &mut Context<Self>) {
        self.save_error = settings::save(&self.settings)
            .err()
            .map(|failure| SharedString::from(failure.to_string()));
        cx.notify();
    }

    fn set_cursor_style(&mut self, style: CursorStyle, cx: &mut Context<Self>) {
        self.settings.cursor_style = style;
        self.save_settings(cx);
    }

    fn toggle_cursor_blink(&mut self, cx: &mut Context<Self>) {
        self.settings.cursor_blink = !self.settings.cursor_blink;
        self.save_settings(cx);
    }

    fn nudge_columns(&mut self, delta: i32, cx: &mut Context<Self>) {
        let next = i32::from(self.settings.columns) + delta;
        self.settings.columns = next.clamp(
            i32::from(settings::MIN_COLUMNS),
            i32::from(settings::MAX_COLUMNS),
        ) as u16;
        self.save_settings(cx);
    }

    fn nudge_rows(&mut self, delta: i32, cx: &mut Context<Self>) {
        let next = i32::from(self.settings.rows) + delta;
        self.settings.rows =
            next.clamp(i32::from(settings::MIN_ROWS), i32::from(settings::MAX_ROWS)) as u16;
        self.save_settings(cx);
    }

    fn set_shell_exit_behavior(&mut self, behavior: ShellExitBehavior, cx: &mut Context<Self>) {
        self.settings.when_shell_exits = behavior;
        self.save_settings(cx);
    }

    fn set_new_window_directory(
        &mut self,
        directory: NewWindowWorkingDirectory,
        cx: &mut Context<Self>,
    ) {
        self.settings.new_window_directory = directory;
        self.save_settings(cx);
    }

    fn set_ask_before_closing(&mut self, value: AskBeforeClosing, cx: &mut Context<Self>) {
        self.settings.ask_before_closing = value;
        self.save_settings(cx);
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

fn text_checkbox(
    id: &'static str,
    label: &'static str,
    checked: bool,
    cx: &mut Context<SettingsView>,
    edit: fn(&mut settings::Settings, bool),
) -> impl IntoElement {
    let view = cx.entity();
    div()
        .px_3()
        .pb_2()
        .child(
            Checkbox::new(id)
                .label(label)
                .checked(checked)
                .on_change(move |value, _, cx| {
                    view.update(cx, |this, cx| {
                        edit(&mut this.settings, *value);
                        this.save_settings(cx);
                    });
                }),
        )
}

/// One row of an exclusive-choice list (Profile, Cursor style, Shell exit
/// behaviour, New-window directory): a label, a leading swatch when given
/// one, and a trailing ✓ on the selected row.
fn choice_row(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    first: bool,
    swatch: Option<u32>,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let label: SharedString = label.into();
    div()
        .id(id)
        .role(Role::RadioButton)
        .aria_label(label.clone())
        .aria_selected(selected)
        .flex()
        .items_center()
        .gap_2()
        .h(px(30.0))
        .px_2()
        .text_size(px(12.0))
        .text_color(rmac_ui::mac::text())
        .when(!first, |row| {
            row.border_t_1().border_color(rmac_ui::mac::separator())
        })
        .hover(|hovered| hovered.bg(rmac_ui::mac::hover()))
        .when_some(swatch, |row, color| {
            row.child(
                div()
                    .w(px(14.0))
                    .h(px(14.0))
                    .rounded(px(rmac_ui::mac::radius_menu_item()))
                    .border_1()
                    .border_color(rmac_ui::mac::separator())
                    .bg(gpui::rgb(color)),
            )
        })
        .child(div().flex_1().child(label))
        .when(selected, |row| {
            row.child(div().text_color(rmac_ui::mac::accent()).child("✓"))
        })
        .on_click(on_click)
}

/// A small named colour swatch, used by `render_colour_effects` for the
/// Text/Bold Text/Selection/Cursor wells and the 16-colour ANSI grid — all
/// real values read from the active default profile, not placeholders.
fn colour_label(label: &'static str, colour: u32) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .child(
            div()
                .w(px(18.0))
                .h(px(18.0))
                .rounded(px(rmac_ui::mac::radius_menu_item()))
                .border_1()
                .border_color(rmac_ui::mac::separator())
                .bg(gpui::rgb(colour)),
        )
        .child(
            div()
                .text_size(px(9.0))
                .text_color(rmac_ui::mac::text_tertiary())
                .child(label),
        )
}

fn ansi_row(label: &'static str, colours: &[u32]) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .w(px(40.0))
                .text_size(px(10.0))
                .text_color(rmac_ui::mac::text_tertiary())
                .child(label),
        )
        .children(colours.iter().map(|colour| {
            div()
                .w(px(14.0))
                .h(px(14.0))
                .rounded(px(rmac_ui::mac::radius_menu_item()))
                .border_1()
                .border_color(rmac_ui::mac::separator())
                .bg(gpui::rgb(*colour))
        }))
}

/// Terminal ▸ Settings… ▸ Text's "Colour & Effects" group: the default
/// profile's Text/Bold Text/Selection/Cursor colour wells and its 16-colour
/// ANSI grid (Normal/Bright rows). View-only here — each profile already
/// ships its full palette (`profiles.rs`); editing a single swatch without
/// turning it into a user-defined profile is future work, not stubbed out.
fn render_colour_effects(profile: &profiles::Profile, bright_bold_text: bool) -> impl IntoElement {
    let bold_text_colour = if bright_bold_text {
        profiles::brighten(profile.fg)
    } else {
        profile.fg
    };
    div()
        .px_3()
        .pb_3()
        .v_flex()
        .gap_2()
        .child(
            div()
                .flex()
                .gap_4()
                .child(colour_label("Text", profile.fg))
                .child(colour_label("Bold Text", bold_text_colour))
                .child(colour_label("Selection", profile.selection))
                .child(colour_label("Cursor", profile.cursor)),
        )
        .child(
            div()
                .pt_1()
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rmac_ui::mac::text_secondary())
                .child("ANSI Colours"),
        )
        .child(
            div()
                .v_flex()
                .gap_1()
                .child(ansi_row("Normal", &profile.ansi[0..8]))
                .child(ansi_row("Bright", &profile.ansi[8..16])),
        )
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
                    .child(section_label("Profiles"))
                    .child(
                        div().px_3().v_flex().child(
                            div()
                                .rounded(px(rmac_ui::mac::radius_control()))
                                .border_1()
                                .border_color(rmac_ui::mac::separator())
                                .overflow_hidden()
                                .children(PROFILES.iter().enumerate().map(|(index, _)| {
                                    let profile = profiles::resolved(index);
                                    choice_row(
                                        ("settings-profile", index),
                                        profile.name,
                                        index == default_profile,
                                        index == 0,
                                        Some(profile.bg),
                                        cx.listener(move |this, _, _, cx| {
                                            this.set_default_profile(index, cx);
                                        }),
                                    )
                                })),
                        ),
                    )
                    .child(section_label("Font"))
                    .child(
                        div()
                            .px_3()
                            .pb_3()
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
                    .child(section_label("Text"))
                    .child(text_checkbox(
                        "settings-use-bold-fonts",
                        "Use bold fonts",
                        self.settings.use_bold_fonts,
                        cx,
                        |settings, value| settings.use_bold_fonts = value,
                    ))
                    .child(text_checkbox(
                        "settings-bright-bold-text",
                        "Use bright colours for bold text",
                        self.settings.bright_bold_text,
                        cx,
                        |settings, value| settings.bright_bold_text = value,
                    ))
                    .child(text_checkbox(
                        "settings-display-ansi-colours",
                        "Display ANSI colours",
                        self.settings.display_ansi_colours,
                        cx,
                        |settings, value| settings.display_ansi_colours = value,
                    ))
                    .child(section_label("Cursor"))
                    .child(
                        div().px_3().v_flex().child(
                            div()
                                .rounded(px(rmac_ui::mac::radius_control()))
                                .border_1()
                                .border_color(rmac_ui::mac::separator())
                                .overflow_hidden()
                                .children(CursorStyle::ALL.into_iter().enumerate().map(
                                    |(index, style)| {
                                        choice_row(
                                            ("settings-cursor-style", index),
                                            style.label(),
                                            style == self.settings.cursor_style,
                                            index == 0,
                                            None,
                                            cx.listener(move |this, _, _, cx| {
                                                this.set_cursor_style(style, cx);
                                            }),
                                        )
                                    },
                                )),
                        ),
                    )
                    .child(
                        div()
                            .id("settings-cursor-blink")
                            .px_3()
                            .pb_3()
                            .mt_2()
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_size(px(12.0))
                            .text_color(rmac_ui::mac::text())
                            .child("Blink cursor")
                            .child(if self.settings.cursor_blink {
                                "On"
                            } else {
                                "Off"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_cursor_blink(cx))),
                    )
                    .child(section_label("Colour & Effects"))
                    .child(render_colour_effects(
                        profiles::resolved(default_profile),
                        self.settings.bright_bold_text,
                    ))
                    .child(section_label("Window"))
                    .child(
                        div()
                            .px_3()
                            .pb_3()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(12.0))
                            .text_color(rmac_ui::mac::text())
                            .child("Size")
                            .child(
                                rmac_ui::Button::new("settings-cols-smaller", "−")
                                    .small()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.nudge_columns(-1, cx)),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(32.0))
                                    .text_center()
                                    .child(SharedString::from(self.settings.columns.to_string())),
                            )
                            .child(
                                rmac_ui::Button::new("settings-cols-bigger", "+")
                                    .small()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.nudge_columns(1, cx)),
                                    ),
                            )
                            .child(div().text_color(rmac_ui::mac::text_tertiary()).child("×"))
                            .child(
                                rmac_ui::Button::new("settings-rows-smaller", "−")
                                    .small()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.nudge_rows(-1, cx)),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(32.0))
                                    .text_center()
                                    .child(SharedString::from(self.settings.rows.to_string())),
                            )
                            .child(
                                rmac_ui::Button::new("settings-rows-bigger", "+")
                                    .small()
                                    .on_click(cx.listener(|this, _, _, cx| this.nudge_rows(1, cx))),
                            )
                            .child(
                                div()
                                    .text_color(rmac_ui::mac::text_tertiary())
                                    .child("cols × rows"),
                            ),
                    )
                    .child(text_checkbox(
                        "settings-title-window-size",
                        "Show window size in title",
                        self.settings.title_shows_window_size,
                        cx,
                        |settings, value| settings.title_shows_window_size = value,
                    ))
                    .child(section_label("Shell"))
                    .child(
                        div().px_3().v_flex().child(
                            div()
                                .rounded(px(rmac_ui::mac::radius_control()))
                                .border_1()
                                .border_color(rmac_ui::mac::separator())
                                .overflow_hidden()
                                .children(ShellExitBehavior::ALL.into_iter().enumerate().map(
                                    |(index, behavior)| {
                                        choice_row(
                                            ("settings-shell-exit", index),
                                            behavior.label(),
                                            behavior == self.settings.when_shell_exits,
                                            index == 0,
                                            None,
                                            cx.listener(move |this, _, _, cx| {
                                                this.set_shell_exit_behavior(behavior, cx);
                                            }),
                                        )
                                    },
                                )),
                        ),
                    )
                    .child(section_label("General"))
                    .child(
                        div().px_3().v_flex().child(
                            div()
                                .rounded(px(rmac_ui::mac::radius_control()))
                                .border_1()
                                .border_color(rmac_ui::mac::separator())
                                .overflow_hidden()
                                .children(
                                    NewWindowWorkingDirectory::ALL.into_iter().enumerate().map(
                                        |(index, directory)| {
                                            choice_row(
                                                ("settings-new-window-dir", index),
                                                format!(
                                                    "New windows open with: {}",
                                                    directory.label()
                                                ),
                                                directory == self.settings.new_window_directory,
                                                index == 0,
                                                None,
                                                cx.listener(move |this, _, _, cx| {
                                                    this.set_new_window_directory(directory, cx);
                                                }),
                                            )
                                        },
                                    ),
                                ),
                        ),
                    )
                    .child(
                        div()
                            .px_3()
                            .pt_2()
                            .text_size(px(11.0))
                            .text_color(rmac_ui::mac::text_secondary())
                            .child("Ask before closing:"),
                    )
                    .child(
                        div().px_3().pb_3().v_flex().child(
                            div()
                                .rounded(px(rmac_ui::mac::radius_control()))
                                .border_1()
                                .border_color(rmac_ui::mac::separator())
                                .overflow_hidden()
                                .children(AskBeforeClosing::ALL.into_iter().enumerate().map(
                                    |(index, value)| {
                                        choice_row(
                                            ("settings-ask-before-closing", index),
                                            value.label(),
                                            value == self.settings.ask_before_closing,
                                            index == 0,
                                            None,
                                            cx.listener(move |this, _, _, cx| {
                                                this.set_ask_before_closing(value, cx);
                                            }),
                                        )
                                    },
                                )),
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
                    )
                    .when_some(self.save_error.clone(), |scroll, error| {
                        scroll.child(
                            div()
                                .px_3()
                                .pb_3()
                                .text_size(px(11.0))
                                .text_color(rmac_ui::mac::danger())
                                .child(error),
                        )
                    }),
            )
    }
}
