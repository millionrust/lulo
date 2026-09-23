//! Language & Region settings presentation.

use super::*;

mod formats;
mod input_sources;

impl Settings {
    pub(in crate::controller) fn render_language_region(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = Button::new("refresh-language-region", "Refresh")
            .busy(self.locale_busy || self.locale_stream_refreshing)
            .disabled(self.locale_loading || self.locale_busy || self.locale_stream_refreshing)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_locale(cx));
            });
        let Some(snapshot) = &self.locale else {
            return self.pane(vec![
                card(vec![row_base()
                    .child(tile("icons/languages.svg", secondary(), style::ROW_ICON))
                    .child(text_block(
                        "System language and formats".into(),
                        Some("systemd-localed".into()),
                    ))
                    .child(refresh)
                    .into_any_element()]),
                note_card(if self.locale_loading {
                    "Reading authoritative locale state and installed locales…"
                } else {
                    "The system locale service is unavailable. No local fallback controls are shown."
                }),
            ]);
        };

        let language_row = if let Some(editor) = &self.locale_editor {
            let cancel_view = view.clone();
            let apply_view = view.clone();
            row_base()
                .child(tile("icons/languages.svg", accent(), style::ROW_ICON))
                .child(text_block(
                    "Language".into(),
                    Some("Enter an exact locale installed on this computer".into()),
                ))
                .child(div().w(px(180.0)).child(TextField::new(editor).small()))
                .child(
                    Button::new("locale-cancel", "Cancel")
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| settings.cancel_locale_edit(cx));
                        }),
                )
                .child(
                    Button::new("locale-apply", "Apply")
                        .primary()
                        .busy(self.locale_busy)
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            apply_view.update(cx, |settings, cx| settings.submit_locale(cx));
                        }),
                )
                .into_any_element()
        } else {
            let edit_view = view.clone();
            row_base()
                .child(tile("icons/languages.svg", accent(), style::ROW_ICON))
                .child(text_block(
                    "Language".into(),
                    Some("Validated against the system's installed locales".into()),
                ))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child(snapshot.language().to_owned()),
                )
                .child(
                    Button::new("locale-edit", "Edit")
                        .disabled(
                            self.locale_busy
                                || self.region_editor.is_some()
                                || self.x11_layout_editor.is_some(),
                        )
                        .on_click(move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_locale_edit(window, cx);
                            });
                        }),
                )
                .into_any_element()
        };

        let region_row = if let Some(editor) = &self.region_editor {
            let cancel_view = view.clone();
            let apply_view = view.clone();
            row_base()
                .child(tile("icons/globe.svg", accent(), style::ROW_ICON))
                .child(text_block(
                    "Region".into(),
                    Some("Sets date, number, currency, and regional formats".into()),
                ))
                .child(div().w(px(180.0)).child(TextField::new(editor).small()))
                .child(
                    Button::new("region-cancel", "Cancel")
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| settings.cancel_region_edit(cx));
                        }),
                )
                .child(
                    Button::new("region-apply", "Apply")
                        .primary()
                        .busy(self.locale_busy)
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            apply_view.update(cx, |settings, cx| settings.submit_region(cx));
                        }),
                )
                .into_any_element()
        } else {
            let edit_view = view.clone();
            let value = if snapshot.formats_are_mixed() {
                format!("Mixed · {}", snapshot.region_locale())
            } else {
                snapshot.region_locale().to_owned()
            };
            row_base()
                .child(tile("icons/globe.svg", accent(), style::ROW_ICON))
                .child(text_block(
                    "Region".into(),
                    Some("System-wide formats, independent of display language".into()),
                ))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child(value),
                )
                .child(
                    Button::new("region-edit", "Edit")
                        .disabled(
                            self.locale_busy
                                || self.locale_editor.is_some()
                                || self.x11_layout_editor.is_some(),
                        )
                        .on_click(move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_region_edit(window, cx);
                            });
                        }),
                )
                .into_any_element()
        };

        let mut cards = vec![card(vec![language_row, region_row])];
        if let Some(editor) = &self.locale_editor {
            let value = editor.read(cx).value();
            match snapshot.preview_language(value.trim()) {
                Ok(preview) => {
                    cards.push(section_header("Assignments applied to the system"));
                    cards.push(card(
                        preview
                            .iter()
                            .map(|assignment| {
                                value_row(
                                    "icons/settings.svg",
                                    secondary(),
                                    assignment.key.clone().into(),
                                    assignment.value.clone().into(),
                                )
                            })
                            .collect(),
                    ));
                    if preview
                        .iter()
                        .any(|assignment| assignment.key.starts_with("LC_"))
                    {
                        cards.push(note_card(
                            "Existing LC_* format overrides are preserved. Applying changes LANG only.",
                        ));
                    }
                }
                Err(error) => cards.push(note_card(error.to_string())),
            }
        }
        if let Some(editor) = &self.region_editor {
            let value = editor.read(cx).value();
            match snapshot.preview_region(value.trim()) {
                Ok(_) => cards.push(note_card(
                    "Applying changes regional date, number, currency, paper, address, telephone, and measurement formats without changing the display language or message locale.",
                )),
                Err(error) => cards.push(note_card(error.to_string())),
            }
        }

        self.append_locale_formats(snapshot, &mut cards);
        self.append_locale_input_sources(view.clone(), snapshot, &mut cards);

        let mut authority_rows = vec![row_base()
            .child(tile("icons/refresh-cw.svg", secondary(), style::ROW_ICON))
            .child(text_block(
                "Authoritative state".into(),
                Some(
                    format!(
                        "Live localed changes · {} installed locales",
                        snapshot.installed_locales.len()
                    )
                    .into(),
                ),
            ))
            .child(refresh)
            .into_any_element()];
        if self.locale_revert.is_some() {
            let revert_view = view.clone();
            authority_rows.push(
                row_base()
                    .child(tile("icons/refresh-cw.svg", secondary(), style::ROW_ICON))
                    .child(text_block(
                        "Previous locale assignments".into(),
                        Some("Reverts only if the complete applied state is still current".into()),
                    ))
                    .child(
                        Button::new("locale-revert", "Revert")
                            .busy(self.locale_busy)
                            .disabled(self.locale_busy)
                            .on_click(move |_, _, cx| {
                                revert_view.update(cx, |settings, cx| settings.revert_locale(cx));
                            }),
                    )
                    .into_any_element(),
            );
        }
        cards.push(card(authority_rows));
        if snapshot.installed_locales_truncated {
            cards.push(note_card(
                "The installed locale inventory exceeded the bounded validation list.",
            ));
        }
        cards.push(note_card(
            "New applications and services use an applied locale immediately. Sign out and back in before judging the current desktop session.",
        ));
        self.pane(cards)
    }
}
