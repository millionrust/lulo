//! The note body's own surfaces: its right-click text menu, Edit ▸ Add
//! Link…, Format ▸ Font ▸ Show Fonts / Show Colours, and Format ▸ Maths
//! Results ▸ Suggest Results' inline offer.

use rmac_editor::rich::{self, fonts};

use super::*;

impl NotesView {
    /// Right-click in the body: Notes' text menu, then Writing Tools.
    pub(super) fn open_body_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = gpui::Focusable::focus_handle(self.body.read(cx), cx);
        self.body_menu = Some(rmac_ui::ContextMenuState::open(
            position, &focus, window, cx,
        ));
        cx.notify();
    }

    pub(super) fn render_body_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.body_menu.as_ref()?;
        let editable = self.body_format_editable();
        let state_now = rmac_writing_tools::TextState {
            selected: !self.body_selection(cx).is_empty(),
            editable,
            pasteable: editable && cx.read_from_clipboard().is_some(),
        };
        let menu = rmac_writing_tools::context_menu(
            &rmac_writing_tools::text_menu::NOTES,
            state_now,
            state.position(),
            cx,
        );
        Some(menu.render(state).into_any_element())
    }

    /// Edit ▸ Add Link… (⌘K): ask for the address the selection links to.
    pub(super) fn insert_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.body_format_editable() {
            return;
        }
        let current = self
            .body
            .read(cx)
            .link_at_selection()
            .map(|link| link.to_string())
            .unwrap_or_default();
        self.link_input
            .update(cx, |input, cx| input.set_value(current, window, cx));
        self.link_dialog_open = true;
        cx.notify();
    }

    pub(super) fn confirm_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.link_dialog_open {
            return;
        }
        self.link_dialog_open = false;
        let address = self.link_input.read(cx).value().trim().to_string();
        let target = (!address.is_empty()).then(|| Arc::<str>::from(address.as_str()));
        self.body.update(cx, |editor, cx| {
            editor.set_link(target, cx);
            editor.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn cancel_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.link_dialog_open = false;
        self.focus_body(window, cx);
        cx.notify();
    }

    /// Format ▸ Font ▸ Show Fonts (⌘T).
    pub(super) fn show_fonts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.body_format_editable() {
            return;
        }
        self.fonts_open = !self.fonts_open;
        self.colours_open = false;
        if !self.fonts_open {
            self.focus_body(window, cx);
            cx.notify();
            return;
        }
        let size = self.body.read(cx).style_at_selection().size;
        let size = if size.fract() == 0.0 {
            format!("{size:.0}")
        } else {
            format!("{size:.1}")
        };
        self.font_size_input
            .update(cx, |input, cx| input.set_value(size, window, cx));
        if self.font_families.is_none() {
            cx.spawn(async move |this, cx| {
                let scanned = cx
                    .background_executor()
                    .spawn(async { fonts::installed_font_families() })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    let families = if scanned.is_empty() {
                        let mut names = cx.text_system().all_font_names();
                        names.sort_by_key(|name| name.to_lowercase());
                        names.dedup();
                        names
                    } else {
                        scanned
                    };
                    this.font_families =
                        Some(families.into_iter().map(SharedString::from).collect());
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn set_font_family(&mut self, family: SharedString, cx: &mut Context<Self>) {
        if !self.body_format_editable() {
            return;
        }
        self.body.update(cx, |editor, cx| {
            editor.set_family(Some(Arc::from(family.as_ref())), cx)
        });
    }

    /// Return in the Fonts panel's size field.
    pub(super) fn apply_font_size_field(&mut self, cx: &mut Context<Self>) {
        let Some(size) = fonts::parse_font_size(&self.font_size_input.read(cx).value()) else {
            return;
        };
        if self.body_format_editable() {
            self.body
                .update(cx, |editor, cx| editor.set_size(size, cx));
        }
    }

    fn close_panels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fonts_open = false;
        self.colours_open = false;
        self.focus_body(window, cx);
        cx.notify();
    }

    /// Format ▸ Font ▸ Show Colours (⇧⌘C).
    pub(super) fn show_colours(&mut self, cx: &mut Context<Self>) {
        if !self.body_format_editable() {
            return;
        }
        self.colours_open = !self.colours_open;
        self.fonts_open = false;
        cx.notify();
    }

    fn apply_colour(&mut self, color: Option<rich::Rgb>, cx: &mut Context<Self>) {
        if self.body_format_editable() {
            self.body
                .update(cx, |editor, cx| editor.set_text_color(color, cx));
        }
    }

    fn panel_card(width: f32) -> Div {
        div()
            .w(px(width))
            .p(px(20.0))
            .v_flex()
            .gap_3()
            .rounded(px(rmac_ui::mac::radius_card()))
            .bg(mac::window())
            .border_1()
            .border_color(mac::separator())
            .shadow_xl()
    }

    fn panel_title(title: &'static str) -> Div {
        div()
            .text_size(rmac_ui::text_px(15.0))
            .font_weight(mac::BOLD)
            .child(title)
    }

    /// The open Add Link sheet, Fonts panel or Colours panel.
    pub(super) fn render_format_panels(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use rmac_ui::DialogButtonKind::{Normal, Primary};

        let body_focus = gpui::Focusable::focus_handle(self.body.read(cx), cx);
        if self.link_dialog_open {
            let card = Self::panel_card(360.0)
                .child(Self::panel_title("Add Link"))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text_secondary())
                        .child("Link the selected text to this address."),
                )
                .child(
                    div()
                        .id("link-address")
                        .role(Role::TextInput)
                        .aria_label("Link")
                        .accessible_text_input(&self.link_input, cx)
                        .child(TextField::new(&self.link_input).cleanable(true)),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            rmac_ui::dialog_button("cancel-link", "Cancel", Normal).on_click(
                                cx.listener(|this, _, window, cx| this.cancel_link(window, cx)),
                            ),
                        )
                        .child(
                            rmac_ui::dialog_button("confirm-link", "OK", Primary).on_click(
                                cx.listener(|this, _, window, cx| this.confirm_link(window, cx)),
                            ),
                        ),
                );
            return Some(
                rmac_ui::dialog("link-dialog", card)
                    .restore_focus_to(body_focus)
                    .into_any_element(),
            );
        }
        if self.fonts_open {
            return Some(self.render_fonts_panel(body_focus, cx));
        }
        if self.colours_open {
            return Some(self.render_colours_panel(body_focus, cx));
        }
        None
    }

    fn render_fonts_panel(&self, body_focus: FocusHandle, cx: &mut Context<Self>) -> AnyElement {
        use rmac_ui::DialogButtonKind::Primary;

        let current = self.body.read(cx).style_at_selection().family;
        let mut list = div()
            .id("font-families")
            .role(Role::List)
            .aria_label("Family")
            .h(px(220.0))
            .overflow_y_scroll()
            .border_1()
            .border_color(mac::separator())
            .rounded(px(mac::radius_control()));
        match &self.font_families {
            None => {
                list = list.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_color(mac::text_secondary())
                        .child("Loading fonts…"),
                );
            }
            Some(families) => {
                for (index, family) in families.iter().enumerate() {
                    let selected = current.as_deref() == Some(family.as_ref());
                    let family = family.clone();
                    list = list.child(
                        div()
                            .id(("font-family", index))
                            .role(Role::RadioButton)
                            .aria_label(family.clone())
                            .aria_selected(selected)
                            .h(px(22.0))
                            .px_2()
                            .flex()
                            .items_center()
                            .when(selected, |row| row.bg(mac::text_selection()))
                            .hover(|row| row.bg(mac::hover()))
                            .font_family(family.clone())
                            .child(family.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_font_family(family.clone(), cx)
                            })),
                    );
                }
            }
        }
        let card = Self::panel_card(320.0)
            .child(Self::panel_title("Fonts"))
            .child(list)
            .child(
                div().flex().items_center().gap_2().child("Size").child(
                    div()
                        .id("font-size")
                        .role(Role::TextInput)
                        .aria_label("Size")
                        .accessible_text_input(&self.font_size_input, cx)
                        .w(px(80.0))
                        .child(TextField::new(&self.font_size_input)),
                ),
            )
            .child(
                div().flex().justify_end().child(
                    rmac_ui::dialog_button("fonts-done", "Done", Primary).on_click(
                        cx.listener(|this, _, window, cx| this.close_panels(window, cx)),
                    ),
                ),
            );
        rmac_ui::dialog("fonts-panel", card)
            .restore_focus_to(body_focus)
            .into_any_element()
    }

    fn render_colours_panel(&self, body_focus: FocusHandle, cx: &mut Context<Self>) -> AnyElement {
        use rmac_ui::DialogButtonKind::{Normal, Primary};

        let current = self.body.read(cx).style_at_selection().color;
        let mut grid = div()
            .flex()
            .flex_wrap()
            .gap(px(4.0))
            .w(px(8.0 * 28.0 + 7.0 * 4.0));
        for (name, color) in rich::palette::CRAYONS {
            let selected = current == Some(color);
            grid = grid.child(
                div()
                    .id(SharedString::from(format!("crayon-{name}")))
                    .role(Role::RadioButton)
                    .aria_label(name)
                    .aria_selected(selected)
                    .w(px(28.0))
                    .h(px(20.0))
                    .rounded(px(mac::radius_control()))
                    .bg(rich::palette::to_hsla(color))
                    .border_1()
                    .border_color(if selected {
                        mac::system_blue()
                    } else {
                        mac::separator()
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.apply_colour(Some(color), cx))),
            );
        }
        let card = Self::panel_card(296.0)
            .child(Self::panel_title("Colours"))
            .child(grid)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(
                        rmac_ui::dialog_button("colours-automatic", "Automatic", Normal)
                            .on_click(cx.listener(|this, _, _, cx| this.apply_colour(None, cx))),
                    )
                    .child(
                        rmac_ui::dialog_button("colours-done", "Done", Primary).on_click(
                            cx.listener(|this, _, window, cx| this.close_panels(window, cx)),
                        ),
                    ),
            );
        rmac_ui::dialog("colours-panel", card)
            .restore_focus_to(body_focus)
            .into_any_element()
    }

    /// Suggest Results' offer, just under the `=` it answers.
    pub(super) fn render_maths_suggestion(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let suggestion = self.maths_suggestion.as_ref()?;
        let caret = self.body.read(cx).caret_bounds()?;
        let label: SharedString = format!("Suggested result {}", suggestion.result).into();
        Some(
            gpui::deferred(
                gpui::anchored()
                    .position(gpui::point(caret.origin.x, caret.origin.y + caret.size.height + px(4.0)))
                    .child(
                        div()
                            .id("maths-suggestion")
                            .role(Role::Tooltip)
                            .aria_label(label)
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded(px(mac::radius_control()))
                            .bg(mac::window())
                            .border_1()
                            .border_color(mac::separator())
                            .shadow_md()
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(BODY_SIZE))
                                    .font_weight(mac::SEMIBOLD)
                                    .text_color(notes_accent())
                                    .child(suggestion.result.clone()),
                            )
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(11.0))
                                    .text_color(mac::text_secondary())
                                    .child("Return to insert"),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    if this.accept_maths_suggestion(cx) {
                                        this.focus_body(window, cx);
                                    }
                                }),
                            ),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}
