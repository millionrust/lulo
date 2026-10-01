//! Text Editor ▸ Settings…: defaults for new plain-text documents.

use gpui::{
    div, px, App, AppContext as _, Context, FocusHandle, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Role, StatefulInteractiveElement as _, Styled as _,
    Window, WindowHandle,
};
use rmac_ui::{Button, Checkbox, Root, StyledExt as _};

use crate::{
    document::TextEncoding,
    settings::{self, Settings},
};

const WIDTH: f32 = 439.0;
const HEIGHT: f32 = 676.0;

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
    let options = rmac_ui::window_options_for_app(rmac_ui::app_id::TEXT_EDITOR, WIDTH, HEIGHT, cx);
    match cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        window.set_window_title("Settings");
        let view = cx.new(SettingsView::new);
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        Ok(handle) => OPEN.with(|open| open.set(Some(handle))),
        Err(error) => eprintln!("Text Editor could not open Settings: {error}"),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    NewDocument,
    OpenAndSave,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Self::NewDocument => "New Document",
            Self::OpenAndSave => "Open and Save",
        }
    }
}

struct SettingsView {
    focus: FocusHandle,
    tab: Tab,
    settings: Settings,
}

impl SettingsView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            tab: Tab::NewDocument,
            settings: settings::current(),
        }
    }

    fn edit(&mut self, change: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        self.settings = settings::update(change);
        cx.notify();
    }

    fn section_label(label: &'static str) -> impl IntoElement {
        div()
            .pt_3()
            .pb_1()
            .text_size(px(12.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rmac_ui::mac::text_secondary())
            .child(label)
    }

    fn number_row(
        id: &'static str,
        label: &'static str,
        value: String,
        unit: &'static str,
        decrease: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        increase: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_2()
            .py_1()
            .child(div().w(px(72.0)).child(label))
            .child(
                Button::new(format!("{id}-decrease"), "−")
                    .small()
                    .on_click(cx.listener(move |this, _, _, cx| decrease(this, cx))),
            )
            .child(
                div()
                    .id(format!("{id}-value"))
                    .role(Role::TextInput)
                    .aria_label(label)
                    .aria_value(value.clone())
                    .w(px(42.0))
                    .text_center()
                    .child(value),
            )
            .child(
                Button::new(format!("{id}-increase"), "+")
                    .small()
                    .on_click(cx.listener(move |this, _, _, cx| increase(this, cx))),
            )
            .child(unit)
    }

    fn checkbox_row(
        id: &'static str,
        label: &'static str,
        checked: bool,
        on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div().py_1().child(
            Checkbox::new(id)
                .label(label)
                .checked(checked)
                .on_change(move |value, window, cx| on_change(value, window, cx)),
        )
    }

    fn encoding_row(&self, encoding: TextEncoding, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.settings.default_encoding == encoding;
        div()
            .id(format!("settings-encoding-{}", encoding.label()))
            .role(Role::RadioButton)
            .aria_label(encoding.label())
            .aria_selected(selected)
            .flex()
            .items_center()
            .justify_between()
            .h(px(30.0))
            .px_2()
            .hover(|hovered| hovered.bg(rmac_ui::mac::hover()))
            .child(encoding.label())
            .child(if selected { "✓" } else { "" })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.edit(|settings| settings.default_encoding = encoding, cx);
            }))
    }

    fn render_new_document(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .child(Self::section_label("Format"))
            .child(
                div()
                    .id("settings-plain-text")
                    .role(Role::RadioButton)
                    .aria_label("Plain text")
                    .aria_selected(true)
                    .child("✓  Plain text"),
            )
            .child(Self::section_label("Window Size"))
            .child(Self::number_row(
                "settings-width",
                "Width:",
                self.settings.width_chars.to_string(),
                "characters",
                |this, cx| {
                    this.edit(
                        |s| s.width_chars = s.width_chars.saturating_sub(1).max(40),
                        cx,
                    )
                },
                |this, cx| {
                    this.edit(
                        |s| s.width_chars = s.width_chars.saturating_add(1).min(240),
                        cx,
                    )
                },
                cx,
            ))
            .child(Self::number_row(
                "settings-height",
                "Height:",
                self.settings.height_lines.to_string(),
                "lines",
                |this, cx| {
                    this.edit(
                        |s| s.height_lines = s.height_lines.saturating_sub(1).max(10),
                        cx,
                    )
                },
                |this, cx| {
                    this.edit(
                        |s| s.height_lines = s.height_lines.saturating_add(1).min(100),
                        cx,
                    )
                },
                cx,
            ))
            .child(Self::section_label("Font"))
            .child("Plain text font:")
            .child(Self::number_row(
                "settings-font-size",
                "Size:",
                self.settings.font_size.to_string(),
                "pt",
                |this, cx| this.edit(|s| s.font_size = s.font_size.saturating_sub(1).max(8), cx),
                |this, cx| this.edit(|s| s.font_size = s.font_size.saturating_add(1).min(32), cx),
                cx,
            ))
            .child(Self::section_label("Options"))
            .child(Self::checkbox_row(
                "settings-wrap-to-page",
                "Wrap to page",
                self.settings.wrap_to_page,
                cx.listener(|this, value: &bool, _, cx| {
                    this.edit(|settings| settings.wrap_to_page = *value, cx);
                }),
            ))
            .child(
                div()
                    .pt_2()
                    .text_size(px(11.0))
                    .text_color(rmac_ui::mac::text_secondary())
                    .child("These defaults apply to new document windows."),
            )
    }

    fn render_open_and_save(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .child(Self::section_label("Plain Text Encoding"))
            .child("New documents use:")
            .child(
                div()
                    .mt_2()
                    .border_1()
                    .border_color(rmac_ui::mac::separator())
                    .rounded(px(rmac_ui::mac::radius_control()))
                    .overflow_hidden()
                    .children(
                        [
                            TextEncoding::Utf8,
                            TextEncoding::Utf8Bom,
                            TextEncoding::Utf16Le,
                            TextEncoding::Utf16Be,
                        ]
                        .into_iter()
                        .map(|encoding| self.encoding_row(encoding, cx)),
                    ),
            )
            .child(
                div()
                    .pt_2()
                    .text_size(px(11.0))
                    .text_color(rmac_ui::mac::text_secondary())
                    .child("Opening a file still detects its encoding from its contents."),
            )
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.tab {
            Tab::NewDocument => self.render_new_document(cx).into_any_element(),
            Tab::OpenAndSave => self.render_open_and_save(cx).into_any_element(),
        };
        div()
            .track_focus(&self.focus)
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
                    .child("Settings"),
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rmac_ui::mac::separator())
                    .children([Tab::NewDocument, Tab::OpenAndSave].into_iter().map(|tab| {
                        Button::new(format!("settings-tab-{}", tab.label()), tab.label())
                            .small()
                            .selected(self.tab == tab)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.tab = tab;
                                cx.notify();
                            }))
                    })),
            )
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_4()
                    .child(body),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(rmac_ui::mac::separator())
                    .child(
                        Button::new("settings-restore-defaults", "Restore All Defaults")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.edit(|settings| *settings = Settings::default(), cx);
                            })),
                    ),
            )
    }
}
