//! Text Editor ▸ Settings…: defaults for new plain-text documents.

use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, Context, Entity, FocusHandle,
    FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, WindowHandle,
};
use rmac_ui::{
    AccessibleTextInput as _, Button, Checkbox, InputEvent, InputState, PopUpButton, PopupMenuItem,
    Root, StyledExt as _, TextField,
};

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
    // Not `window_options_for_app`: Settings would then inherit whatever
    // size a document window last saved under the same app_id (UIA-09).
    let options =
        rmac_ui::window_options_for_panel(rmac_ui::app_id::TEXT_EDITOR, WIDTH, HEIGHT, cx);
    match cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        window.set_window_title("Settings");
        let view = cx.new(|cx| SettingsView::new(window, cx));
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
    author_input: Entity<InputState>,
    organisation_input: Entity<InputState>,
    copyright_input: Entity<InputState>,
}

impl SettingsView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = settings::current();
        let author_input =
            cx.new(|cx| InputState::new(window, cx).default_value(settings.author_default.clone()));
        let organisation_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(settings.organisation_default.clone())
        });
        let copyright_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(settings.copyright_default.clone())
        });
        cx.subscribe(&author_input, |this, input, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                let value = input.read(cx).value().to_string();
                this.edit(|settings| settings.author_default = value, cx);
            }
        })
        .detach();
        cx.subscribe(
            &organisation_input,
            |this, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    let value = input.read(cx).value().to_string();
                    this.edit(|settings| settings.organisation_default = value, cx);
                }
            },
        )
        .detach();
        cx.subscribe(&copyright_input, |this, input, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                let value = input.read(cx).value().to_string();
                this.edit(|settings| settings.copyright_default = value, cx);
            }
        })
        .detach();
        Self {
            focus: cx.focus_handle(),
            tab: Tab::NewDocument,
            settings,
            author_input,
            organisation_input,
            copyright_input,
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

    /// Settings ▸ New Document ▸ Properties (TXT-SETTINGS-001/003/006): a
    /// labelled single-line text field, the same row shape `number_row`
    /// uses but editable free text instead of a stepper.
    fn text_row(
        label: &'static str,
        input: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_2()
            .py_1()
            .child(div().w(px(96.0)).child(label))
            .child(
                div()
                    .id(SharedString::from(label))
                    .flex_1()
                    .role(Role::TextInput)
                    .aria_label(label.trim_end_matches(':'))
                    .accessible_text_input(input, cx)
                    .child(TextField::new(input)),
            )
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

    fn render_new_document(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let plain_selected = !self.settings.rich_text_default;
        let rich_selected = self.settings.rich_text_default;
        div()
            .child(Self::section_label("Format"))
            .child(
                // Format ▸ radio row: Plain text (TXT-SETTINGS-013's
                // sibling), the default new documents open as unless Rich
                // text is chosen below.
                div()
                    .id("settings-format-plain-text")
                    .role(Role::RadioButton)
                    .aria_label("Plain text")
                    .aria_selected(plain_selected)
                    .flex()
                    .items_center()
                    .gap_2()
                    .py_1()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.edit(|settings| settings.rich_text_default = false, cx);
                    }))
                    .child(if plain_selected { "✓" } else { " " })
                    .child("Plain text"),
            )
            .child(
                // Format ▸ radio row: Rich text (TXT-SETTINGS-013), the
                // default [`settings::Settings::rich_text_default`] a new
                // document window opens with (see
                // `EditorView::new_with_path`).
                div()
                    .id("settings-format-rich-text")
                    .role(Role::RadioButton)
                    .aria_label("Rich text")
                    .aria_selected(rich_selected)
                    .flex()
                    .items_center()
                    .gap_2()
                    .py_1()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.edit(|settings| settings.rich_text_default = true, cx);
                    }))
                    .child(if rich_selected { "✓" } else { " " })
                    .child("Rich text"),
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
            .child("Rich text font:")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .py_1()
                    .child(format!(
                        "{} {}",
                        self.settings.rich_text_font.label(),
                        self.settings.rich_text_font_size
                    ))
                    .child(
                        Button::new("settings-rich-text-font-change", "Change…")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.edit(
                                    |prefs| {
                                        prefs.rich_text_font = match prefs.rich_text_font {
                                            settings::RichTextFont::Inter => {
                                                settings::RichTextFont::JetBrainsMono
                                            }
                                            settings::RichTextFont::JetBrainsMono => {
                                                settings::RichTextFont::Inter
                                            }
                                        };
                                    },
                                    cx,
                                );
                            })),
                    ),
            )
            .child(Self::number_row(
                "settings-rich-text-font-size",
                "Size:",
                self.settings.rich_text_font_size.to_string(),
                "pt",
                |this, cx| {
                    this.edit(
                        |s| s.rich_text_font_size = s.rich_text_font_size.saturating_sub(1).max(8),
                        cx,
                    )
                },
                |this, cx| {
                    this.edit(
                        |s| s.rich_text_font_size = s.rich_text_font_size.saturating_add(1).min(32),
                        cx,
                    )
                },
                cx,
            ))
            .child(Self::section_label("Properties"))
            .child(Self::text_row("Author:", &self.author_input, cx))
            .child(Self::text_row(
                "Organisation:",
                &self.organisation_input,
                cx,
            ))
            .child(Self::text_row("Copyright:", &self.copyright_input, cx))
            .child(Self::section_label("Options"))
            .child(
                div()
                    .pb_1()
                    .text_size(px(11.0))
                    .text_color(rmac_ui::mac::text_secondary())
                    .child("Use the Format menu to choose settings for an open document."),
            )
            .child(
                div()
                    .pb_1()
                    .text_size(px(11.0))
                    .text_color(rmac_ui::mac::text_secondary())
                    .child("Document properties are saved only with rich text files. Choose File > Show Properties to change the properties for an open document."),
            )
            .child(Self::checkbox_row(
                "settings-wrap-to-page",
                "Wrap to page",
                self.settings.wrap_to_page,
                cx.listener(|this, value: &bool, _, cx| {
                    this.edit(|settings| settings.wrap_to_page = *value, cx);
                }),
            ))
            .child(Self::checkbox_row(
                "settings-show-ruler",
                "Show ruler",
                self.settings.show_ruler_default,
                cx.listener(|this, value: &bool, _, cx| {
                    this.edit(|settings| settings.show_ruler_default = *value, cx);
                }),
            ))
            // TXT-SETTINGS-003/004/006/007/012..015/017: the starting
            // state for every new document window's own Edit ▸ Spelling
            // and Grammar / Substitutions toggles — real settings a new
            // window actually seeds itself from (`EditorView::new_with_path`),
            // not a cosmetic duplicate of the per-window menu.
            .child(Self::section_label("Spelling"))
            .child(Self::checkbox_row(
                "settings-check-spelling-while-typing",
                "Check spelling as you type",
                self.settings.check_spelling_while_typing_default,
                cx.listener(|this, value: &bool, _, cx| {
                    this.edit(
                        |settings| settings.check_spelling_while_typing_default = *value,
                        cx,
                    );
                }),
            ))
            .child(Self::checkbox_row(
                "settings-check-grammar-with-spelling",
                "Check grammar with spelling",
                self.settings.check_grammar_with_spelling_default,
                cx.listener(|this, value: &bool, _, cx| {
                    this.edit(
                        |settings| settings.check_grammar_with_spelling_default = *value,
                        cx,
                    );
                }),
            ))
            .child(Self::checkbox_row(
                "settings-correct-spelling-automatically",
                "Correct spelling automatically",
                self.settings.correct_spelling_automatically_default,
                cx.listener(|this, value: &bool, _, cx| {
                    this.edit(
                        |settings| settings.correct_spelling_automatically_default = *value,
                        cx,
                    );
                }),
            ))
            .child(Self::section_label("Substitutions"))
            .child(Self::checkbox_row(
                "settings-smart-copy-paste",
                "Smart copy/paste",
                self.settings.smart_copy_paste_default,
                cx.listener(|this, value: &bool, _, cx| {
                    this.edit(|settings| settings.smart_copy_paste_default = *value, cx);
                }),
            ))
            .child(Self::checkbox_row(
                "settings-smart-quotes",
                "Smart quotes",
                self.settings.smart_quotes_default,
                cx.listener(|this, value: &bool, _, cx| {
                    this.edit(|settings| settings.smart_quotes_default = *value, cx);
                }),
            ))
            .child(Self::checkbox_row(
                "settings-smart-dashes",
                "Smart dashes",
                self.settings.smart_dashes_default,
                cx.listener(|this, value: &bool, _, cx| {
                    this.edit(|settings| settings.smart_dashes_default = *value, cx);
                }),
            ))
            .child(Self::checkbox_row(
                "settings-smart-links",
                "Smart links",
                self.settings.smart_links_default,
                cx.listener(|this, value: &bool, _, cx| {
                    this.edit(|settings| settings.smart_links_default = *value, cx);
                }),
            ))
            .child(Self::checkbox_row(
                "settings-data-detectors",
                "Data detectors",
                self.settings.data_detectors_default,
                cx.listener(|this, value: &bool, _, cx| {
                    this.edit(|settings| settings.data_detectors_default = *value, cx);
                }),
            ))
            .child(Self::checkbox_row(
                "settings-text-replacement",
                "Text replacement",
                self.settings.text_replacement_default,
                cx.listener(|this, value: &bool, _, cx| {
                    this.edit(|settings| settings.text_replacement_default = *value, cx);
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
        let view = cx.entity();
        let current = self.settings.default_encoding;
        div()
            .child(Self::section_label("Plain Text Encoding"))
            .child("New documents use:")
            .child(
                // A pop-up, not a list showing every choice at once
                // (UIA-09 leftovers — matches the Mac's `AXPopUpButton`
                // encoding controls, `win-settings-open-save-*` audit
                // capture).
                div().mt_2().w_full().child(
                    PopUpButton::new("settings-default-encoding", current.label())
                        .w_full()
                        .dropdown_menu(move |menu, _, _| {
                            [
                                TextEncoding::Utf8,
                                TextEncoding::Utf8Bom,
                                TextEncoding::Utf16Le,
                                TextEncoding::Utf16Be,
                            ]
                            .into_iter()
                            .fold(menu, |menu, encoding| {
                                let view = view.clone();
                                menu.item(
                                    PopupMenuItem::new(encoding.label())
                                        .checked(encoding == current)
                                        .on_click(move |_, _, cx| {
                                            view.update(cx, |this, cx| {
                                                this.edit(
                                                    |settings| settings.default_encoding = encoding,
                                                    cx,
                                                );
                                            });
                                        }),
                                )
                            })
                        }),
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
                    .id("settings-tabs")
                    .role(Role::TabList)
                    .flex()
                    .justify_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rmac_ui::mac::separator())
                    .children([Tab::NewDocument, Tab::OpenAndSave].into_iter().map(|tab| {
                        let selected = self.tab == tab;
                        // A plain, directly-styled tab, not `rmac_ui::Button`'s
                        // `.selected()` (the highlight stayed on New Document
                        // after clicking Open and Save — UIA-09): fill and
                        // text colour both read `selected` fresh every
                        // render, the same proven div-based pattern this
                        // window's own Format/encoding radio rows already use
                        // below, rather than a second implementation of
                        // toggle styling.
                        div()
                            .id(SharedString::from(format!("settings-tab-{}", tab.label())))
                            .role(Role::Tab)
                            .aria_label(tab.label())
                            .aria_selected(selected)
                            .cursor_pointer()
                            .px_3()
                            .py_1()
                            .rounded(px(rmac_ui::mac::radius_control()))
                            .when(selected, |button| {
                                button
                                    .bg(rmac_ui::mac::control_fill_hover())
                                    .text_color(rmac_ui::mac::text())
                            })
                            .when(!selected, |button| {
                                button
                                    .text_color(rmac_ui::mac::text_secondary())
                                    .hover(|hovered| hovered.bg(rmac_ui::mac::hover()))
                            })
                            .child(tab.label())
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
                    .justify_start()
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
