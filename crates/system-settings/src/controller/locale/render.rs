//! Language & Region settings presentation.

use super::*;

mod formats;
mod input_sources;

impl Settings {
    /// macOS 26 Language & Region (design-lab/settings.html): Preferred
    /// Languages as a one-row list with Edit…, then the Region group opened
    /// by the centred format examples, then the Keyboard section; Refresh and
    /// Revert sit under the last group.
    pub(in crate::controller) fn render_language_region(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = push_button("refresh-language-region", "Refresh")
            .busy(self.locale_busy || self.locale_stream_refreshing)
            .disabled(self.locale_loading || self.locale_busy || self.locale_stream_refreshing)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_locale(cx));
            })
            .into_any_element();
        let Some(snapshot) = &self.locale else {
            return self.pane(vec![
                note_card(if self.locale_loading {
                    "Reading the system language and formats…"
                } else {
                    "The system locale service is unavailable."
                }),
                footer_buttons(vec![refresh]),
            ]);
        };

        // Preferred Languages: the list row, then Edit… (or the editor).
        let mut languages = group().child(group_heading("Preferred Languages", None));
        if let Some(editor) = &self.locale_editor {
            let cancel_view = view.clone();
            let apply_view = view.clone();
            languages = languages
                .child(
                    row_base()
                        .child(text_block(
                            "Language".into(),
                            Some("Enter a locale installed on this computer".into()),
                        ))
                        .child(div().w(px(200.0)).child(TextField::new(editor).small())),
                )
                .child(button_row(vec![
                    push_button("locale-cancel", "Cancel")
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| settings.cancel_locale_edit(cx));
                        })
                        .into_any_element(),
                    Button::new("locale-apply", "Apply")
                        .primary()
                        .busy(self.locale_busy)
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            apply_view.update(cx, |settings, cx| settings.submit_locale(cx));
                        })
                        .into_any_element(),
                ]));
        } else {
            let edit_view = view.clone();
            languages = languages
                .child(well_row(
                    "preferred-language",
                    false,
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_size(rmac_ui::text_px(13.0))
                        .child(
                            div()
                                .text_color(label())
                                .child(snapshot.language().to_owned()),
                        )
                        .child(div().text_color(secondary()).child("Primary")),
                    None,
                ))
                .child(button_row(vec![push_button("locale-edit", "Edit…")
                    .disabled(
                        self.locale_busy
                            || self.region_editor.is_some()
                            || self.x11_layout_editor.is_some(),
                    )
                    .on_click(move |_, window, cx| {
                        edit_view.update(cx, |settings, cx| {
                            settings.start_locale_edit(window, cx);
                        });
                    })
                    .into_any_element()]));
        }
        let mut cards = vec![languages];
        if let Some(editor) = &self.locale_editor {
            let value = editor.read(cx).value();
            match snapshot.preview_language(value.trim()) {
                Ok(preview) => {
                    cards.push(section_header("Changes applied to the system"));
                    cards.push(card(
                        preview
                            .iter()
                            .map(|assignment| {
                                fact_row(assignment.key.clone(), assignment.value.clone())
                            })
                            .collect(),
                    ));
                    if preview
                        .iter()
                        .any(|assignment| assignment.key.starts_with("LC_"))
                    {
                        cards.push(footnote(
                            "Existing format overrides are kept. Applying changes the language only.",
                        ));
                    }
                }
                Err(error) => cards.push(note_card(error.to_string())),
            }
        }

        // Region: the examples, then the region with Edit… (or the editor).
        let region_row = if let Some(editor) = &self.region_editor {
            let cancel_view = view.clone();
            let apply_view = view.clone();
            row_base()
                .child(text_block(
                    "Region".into(),
                    Some("Sets date, number, currency and measurement formats".into()),
                ))
                .child(div().w(px(160.0)).child(TextField::new(editor).small()))
                .child(
                    push_button("region-cancel", "Cancel")
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
            value_button_row(
                "Region",
                None,
                Some(value.into()),
                Some(
                    push_button("region-edit", "Edit…")
                        .disabled(
                            self.locale_busy
                                || self.locale_editor.is_some()
                                || self.x11_layout_editor.is_some(),
                        )
                        .on_click(move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_region_edit(window, cx);
                            });
                        })
                        .into_any_element(),
                ),
            )
        };
        let mut region = group();
        if let Some(examples) = self.locale_format_examples(snapshot) {
            region = region.child(examples).child(row_separator());
        }
        cards.push(region.child(region_row));
        if let Some(error) = &snapshot.format_preview_error {
            cards.push(footnote(format!(
                "Format examples are unavailable: {error}."
            )));
        }
        if let Some(editor) = &self.region_editor {
            let value = editor.read(cx).value();
            if let Err(error) = snapshot.preview_region(value.trim()) {
                cards.push(note_card(error.to_string()));
            }
        }

        self.append_locale_input_sources(view.clone(), snapshot, &mut cards);

        if snapshot.installed_locales_truncated {
            cards.push(footnote(
                "The installed language list was too long to check completely.",
            ));
        }
        let mut buttons = Vec::new();
        if self.locale_revert.is_some() {
            let revert_view = view.clone();
            buttons.push(
                push_button("locale-revert", "Revert Language & Region")
                    .busy(self.locale_busy)
                    .disabled(self.locale_busy)
                    .on_click(move |_, _, cx| {
                        revert_view.update(cx, |settings, cx| settings.revert_locale(cx));
                    })
                    .into_any_element(),
            );
        }
        buttons.push(refresh);
        cards.push(footer_buttons(buttons));
        self.pane(cards)
    }
}
