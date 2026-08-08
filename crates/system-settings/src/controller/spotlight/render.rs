//! Spotlight settings presentation.

use super::*;

mod indexing_shortcut;
mod privacy;
mod results;

impl Settings {
    pub(in crate::controller) fn render_spotlight(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let revert_view = view.clone();
        let clear_history_view = view.clone();
        let mut cards = vec![div()
            .flex()
            .items_center()
            .justify_between()
            .px_1()
            .pb_1()
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child("rmac Search"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("spotlight-revert", "Revert")
                            .disabled(
                                self.shell_settings_loading
                                    || self.shell_settings_busy
                                    || self.spotlight_revert.is_none(),
                            )
                            .on_click(move |_, _, cx| {
                                revert_view.update(cx, |settings, cx| {
                                    settings.revert_spotlight_change(cx)
                                });
                            }),
                    )
                    .child(
                        Button::new(
                            "spotlight-refresh",
                            if self.shell_settings_busy {
                                "Applying…"
                            } else if self.shell_settings_loading || self.shortcut_status_loading {
                                "Loading…"
                            } else {
                                "Refresh"
                            },
                        )
                        .disabled(self.shell_settings_loading || self.shell_settings_busy)
                        .on_click(move |_, _, cx| {
                            refresh_view.update(cx, |settings, cx| {
                                settings.refresh_shell_settings(false, cx);
                                settings.refresh_shortcut_status(cx);
                            });
                        }),
                    ),
            )];

        cards.push(section_header("Recent documents"));
        cards.push(card(vec![row_base()
            .child(text_block(
                "rmac recent history".into(),
                Some(
                    "Used by Files and Launcher alongside newer desktop XBEL entries; clearing never deletes a document"
                        .into(),
                ),
            ))
            .child(
                Button::new(
                    "spotlight-clear-history",
                    if self.recent_history_busy {
                        "Clearing…"
                    } else {
                        "Clear…"
                    },
                )
                .disabled(self.recent_history_busy || self.recent_history_confirmation)
                .on_click(move |_, _, cx| {
                    clear_history_view.update(cx, |settings, cx| {
                        settings.request_recent_history_clear(cx)
                    });
                }),
            )
            .into_any_element()]));
        if let Some(notice) = &self.recent_history_notice {
            cards.push(note_card(notice.clone()));
        }
        if self.recent_history_confirmation {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(note_card(
                "Clear recent documents from rmac Search? Files are not deleted. rmac will hide existing desktop-history entries, while other applications continue to manage and display their own history.",
            ));
            cards.push(card(vec![row_base()
                .child(div().flex_1())
                .child(
                    Button::new("spotlight-clear-history-cancel", "Cancel").on_click(
                        move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| {
                                settings.cancel_recent_history_clear(cx)
                            });
                        },
                    ),
                )
                .child(
                    rmac_ui::dialog_button(
                        "spotlight-clear-history-confirm",
                        "Clear History",
                        rmac_ui::DialogButtonKind::Destructive,
                    )
                    .disabled(self.recent_history_busy)
                    .on_click(move |_, _, cx| {
                        confirm_view
                            .update(cx, |settings, cx| settings.confirm_recent_history_clear(cx));
                    }),
                )
                .into_any_element()]));
        }

        if self.shell_settings_loading && self.shell_settings.is_none() {
            cards.push(note_card("Loading authoritative search preferences…"));
            return self.pane(cards);
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            cards.push(note_card(
                "The versioned rmac shell-settings authority is unavailable. Search preferences remain unchanged.",
            ));
            return self.pane(cards);
        };
        let settings = &snapshot.settings;
        self.append_spotlight_results(view.clone(), settings, &mut cards);
        self.append_spotlight_privacy(view.clone(), settings, &mut cards);
        self.append_spotlight_indexing_shortcut(view, settings, &mut cards);
        self.pane(cards)
    }
}
