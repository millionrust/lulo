//! System keyboard input-source projection.

use super::*;

impl Settings {
    pub(super) fn append_locale_input_sources(
        &self,
        view: Entity<Self>,
        snapshot: &rmac_locale::Snapshot,
        cards: &mut Vec<Div>,
    ) {
        let keyboard = if snapshot.x11_layout.is_empty() {
            "Not reported by systemd-localed".to_owned()
        } else {
            let mut value = snapshot.x11_layout.clone();
            if !snapshot.x11_variant.is_empty() {
                value.push_str(" · ");
                value.push_str(&snapshot.x11_variant);
            }
            value
        };
        let layout_authority = self.input.keyboard_layout_authority;
        let can_edit_layout = layout_authority
            == rmac_input::KeyboardLayoutAuthority::SystemLocaled
            && snapshot.x11_layouts_error.is_none()
            && !snapshot.installed_x11_layouts.is_empty();
        cards.push(section_header("Input sources"));
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
                    .child(tile("icons/keyboard.svg", accent(), style::ROW_ICON))
                    .child(text_block(
                        "XKB layouts".into(),
                        Some("Comma-separated installed names, in switch order".into()),
                    ))
                    .child(div().w(px(190.0)).child(TextField::new(layout).small()))
                    .into_any_element(),
                row_base()
                    .child(tile("icons/settings.svg", secondary(), style::ROW_ICON))
                    .child(text_block(
                        "Variants".into(),
                        Some("One entry per layout; empty entries are allowed".into()),
                    ))
                    .child(div().w(px(190.0)).child(TextField::new(variant).small()))
                    .into_any_element(),
                row_base()
                    .child(tile("icons/settings.svg", secondary(), style::ROW_ICON))
                    .child(text_block(
                        "Switching options".into(),
                        Some("For example grp:ctrl_space_toggle".into()),
                    ))
                    .child(div().w(px(150.0)).child(TextField::new(options).small()))
                    .child(
                        Button::new("x11-keyboard-cancel", "Cancel")
                            .disabled(self.locale_busy)
                            .on_click(move |_, _, cx| {
                                cancel_view.update(cx, |settings, cx| {
                                    settings.cancel_x11_keyboard_edit(cx);
                                });
                            }),
                    )
                    .child(
                        Button::new("x11-keyboard-apply", "Apply")
                            .primary()
                            .busy(self.locale_busy)
                            .disabled(self.locale_busy)
                            .on_click(move |_, _, cx| {
                                apply_view.update(cx, |settings, cx| {
                                    settings.submit_x11_keyboard(cx);
                                });
                            }),
                    )
                    .into_any_element(),
            ]
        } else {
            let edit_view = view.clone();
            vec![row_base()
                .child(tile("icons/keyboard.svg", accent(), style::ROW_ICON))
                .child(text_block(
                    "Keyboard layouts".into(),
                    Some("systemd-localed default and switch order".into()),
                ))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child(keyboard),
                )
                .child(
                    Button::new("x11-keyboard-edit", "Edit")
                        .disabled(self.locale_busy || !can_edit_layout)
                        .on_click(move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_x11_keyboard_edit(window, cx);
                            });
                        }),
                )
                .into_any_element()]
        };
        if !editing_keyboard {
            keyboard_rows.push(value_row(
                "icons/settings.svg",
                secondary(),
                "Switching options".into(),
                if snapshot.x11_options.is_empty() {
                    "Not configured".into()
                } else {
                    snapshot.x11_options.clone().into()
                },
            ));
        }
        keyboard_rows.push(value_row(
            "icons/keyboard.svg",
            secondary(),
            "Console keymap".into(),
            if snapshot.console_keymap.is_empty() {
                "Not configured".into()
            } else {
                snapshot.console_keymap.clone().into()
            },
        ));
        if self.x11_keyboard_revert.is_some() {
            let revert_view = view.clone();
            keyboard_rows.push(
                row_base()
                    .child(tile("icons/refresh-cw.svg", secondary(), style::ROW_ICON))
                    .child(text_block(
                        "Previous keyboard layout".into(),
                        Some("Exact model, layouts, variants, and options".into()),
                    ))
                    .child(
                        Button::new("x11-keyboard-revert", "Revert")
                            .busy(self.locale_busy)
                            .disabled(self.locale_busy)
                            .on_click(move |_, _, cx| {
                                revert_view.update(cx, |settings, cx| {
                                    settings.revert_x11_keyboard(cx);
                                });
                            }),
                    )
                    .into_any_element(),
            );
        }
        cards.push(card(keyboard_rows));
        cards.push(note_card(match layout_authority {
            rmac_input::KeyboardLayoutAuthority::SystemLocaled => {
                "niri follows this systemd-localed layout because no explicit XKB override is present. Use the configured XKB switching option to move between multiple layouts."
            }
            rmac_input::KeyboardLayoutAuthority::NiriConfig => {
                "The niri config has an explicit XKB block, so it—not systemd-localed—owns this session's keyboard layout. The system default is read-only here to avoid overriding that choice."
            }
            rmac_input::KeyboardLayoutAuthority::IncludedConfig => {
                "A traversed niri include owns explicit XKB settings, so the systemd-localed default remains read-only here instead of competing with that configuration."
            }
            rmac_input::KeyboardLayoutAuthority::Unavailable => {
                "The active niri keyboard-layout authority could not be verified. The system default remains read-only."
            }
        }));
        if let Some(error) = &snapshot.x11_layouts_error {
            cards.push(note_card(format!(
                "Keyboard layout editing is unavailable: {error}."
            )));
        }
        if snapshot.installed_x11_layouts_truncated {
            cards.push(note_card(
                "The installed XKB layout inventory exceeded the bounded validation list.",
            ));
        }
    }
}
