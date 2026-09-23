//! System keyboard input-source projection.

use super::*;

impl Settings {
    /// Language & Region's "Keyboard" section: the localed layouts with
    /// Edit… (three fields while editing), the switching options and console
    /// keymap, and Revert after a change.
    pub(super) fn append_locale_input_sources(
        &self,
        view: Entity<Self>,
        snapshot: &rmac_locale::Snapshot,
        cards: &mut Vec<Div>,
    ) {
        let layout_authority = self.input.keyboard_layout_authority;
        let can_edit_layout = layout_authority
            == rmac_input::KeyboardLayoutAuthority::SystemLocaled
            && snapshot.x11_layouts_error.is_none()
            && !snapshot.installed_x11_layouts.is_empty();
        cards.push(section_header("Keyboard"));
        let editing_keyboard = self.x11_layout_editor.is_some();
        let mut keyboard_rows = if let (Some(layout), Some(variant), Some(options)) = (
            &self.x11_layout_editor,
            &self.x11_variant_editor,
            &self.x11_options_editor,
        ) {
            let cancel_view = view.clone();
            let apply_view = view.clone();
            vec![
                row_base()
                    .child(text_block(
                        "Layouts".into(),
                        Some("Comma-separated installed names, in switch order".into()),
                    ))
                    .child(div().w(px(190.0)).child(TextField::new(layout).small()))
                    .into_any_element(),
                row_base()
                    .child(text_block(
                        "Variants".into(),
                        Some("One entry per layout; empty entries are allowed".into()),
                    ))
                    .child(div().w(px(190.0)).child(TextField::new(variant).small()))
                    .into_any_element(),
                row_base()
                    .child(text_block(
                        "Switching options".into(),
                        Some("For example grp:ctrl_space_toggle".into()),
                    ))
                    .child(div().w(px(190.0)).child(TextField::new(options).small()))
                    .into_any_element(),
                button_row(vec![
                    push_button("x11-keyboard-cancel", "Cancel")
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| {
                                settings.cancel_x11_keyboard_edit(cx);
                            });
                        })
                        .into_any_element(),
                    Button::new("x11-keyboard-apply", "Apply")
                        .primary()
                        .busy(self.locale_busy)
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            apply_view.update(cx, |settings, cx| {
                                settings.submit_x11_keyboard(cx);
                            });
                        })
                        .into_any_element(),
                ]),
            ]
        } else {
            let edit_view = view.clone();
            vec![value_button_row(
                "Input Sources",
                None,
                Some(locale_keyboard_summary(snapshot).into()),
                Some(
                    push_button("x11-keyboard-edit", "Edit…")
                        .disabled(self.locale_busy || !can_edit_layout)
                        .on_click(move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_x11_keyboard_edit(window, cx);
                            });
                        })
                        .into_any_element(),
                ),
            )]
        };
        if !editing_keyboard {
            keyboard_rows.push(fact_row(
                "Switching options",
                if snapshot.x11_options.is_empty() {
                    "None".to_owned()
                } else {
                    snapshot.x11_options.clone()
                },
            ));
        }
        keyboard_rows.push(fact_row(
            "Console keymap",
            if snapshot.console_keymap.is_empty() {
                "None".to_owned()
            } else {
                snapshot.console_keymap.clone()
            },
        ));
        if self.x11_keyboard_revert.is_some() {
            let revert_view = view.clone();
            keyboard_rows.push(value_button_row(
                "Previous keyboard layout",
                None,
                None,
                Some(
                    push_button("x11-keyboard-revert", "Revert")
                        .busy(self.locale_busy)
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            revert_view.update(cx, |settings, cx| {
                                settings.revert_x11_keyboard(cx);
                            });
                        })
                        .into_any_element(),
                ),
            ));
        }
        cards.push(card(keyboard_rows));
        // Say why Edit… is dimmed when niri, not localed, owns the layout.
        match layout_authority {
            rmac_input::KeyboardLayoutAuthority::SystemLocaled => {}
            rmac_input::KeyboardLayoutAuthority::NiriConfig
            | rmac_input::KeyboardLayoutAuthority::IncludedConfig => {
                cards.push(footnote(
                    "The niri configuration sets this session's keyboard layout, so the system layout is read-only here.",
                ));
            }
            rmac_input::KeyboardLayoutAuthority::Unavailable => {
                cards.push(footnote(
                    "The session's keyboard layout could not be checked, so the system layout is read-only here.",
                ));
            }
        }
        if let Some(error) = &snapshot.x11_layouts_error {
            cards.push(note_card(format!(
                "Keyboard layout editing is unavailable: {error}."
            )));
        }
        if snapshot.installed_x11_layouts_truncated {
            cards.push(footnote(
                "The installed keyboard layout list was too long to check completely.",
            ));
        }
    }
}
