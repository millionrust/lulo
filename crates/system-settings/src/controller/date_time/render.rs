//! Date & Time settings presentation.

use super::*;

impl Settings {
    pub(in crate::controller) fn render_clock_confirmation(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let target = self.clock_confirmation.as_ref()?;
        let current = self
            .time
            .as_ref()
            .map(|snapshot| {
                snapshot.formatted_local_time_at(
                    current_system_time_usec().unwrap_or(snapshot.time_usec),
                )
            })
            .unwrap_or_else(|| "Unavailable".into());
        let target_display = target.display().to_string();
        Some(
            rmac_ui::alert(
                "Set the System Clock?",
                format!(
                    "Current time: {current}\n\nNew time: {target_display}\n\nChanging the clock can affect certificates, file dates, and scheduled work. systemd-timedated also updates the hardware clock, and Linux may ask you to authorize this change."
                ),
                vec![
                    rmac_ui::dialog_button(
                        "clock-change-cancel",
                        "Cancel",
                        rmac_ui::DialogButtonKind::Normal,
                    )
                    .disabled(self.clock_setting)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cancel_clock_confirmation(cx)
                    }))
                    .into_any_element(),
                    rmac_ui::dialog_button(
                        "clock-change-confirm",
                        "Set Clock",
                        rmac_ui::DialogButtonKind::Primary,
                    )
                    .disabled(self.clock_setting)
                    .on_click(cx.listener(|this, _, _, cx| this.confirm_clock_change(cx)))
                    .into_any_element(),
                ],
            )
            .into_any_element(),
        )
    }
    /// macOS 26 Date & Time (design-lab/settings.html): the automatic switch,
    /// the date and time with Set… while automatic time is off, and the time
    /// zone with Set…, each in its own group; Refresh sits under them.
    pub(in crate::controller) fn render_date_time(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = push_button("refresh-date-time", "Refresh")
            .busy(self.time_busy || self.time_stream_refreshing)
            .disabled(self.time_loading || self.time_busy || self.time_stream_refreshing)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_time(cx));
            })
            .into_any_element();
        let Some(snapshot) = &self.time else {
            return self.pane(vec![
                note_card(if self.time_loading {
                    "Reading date and time from systemd-timedated…"
                } else {
                    "The system date and time service is unavailable."
                }),
                footer_buttons(vec![refresh]),
            ]);
        };

        let ntp_view = view.clone();
        let automatic = switch_row(
            "automatic-time",
            "Set time and date automatically",
            (!snapshot.can_ntp).then(|| "No compatible network time service is installed".into()),
            snapshot.ntp_enabled,
            !self.time_busy && snapshot.can_ntp,
            move |enabled, _, cx| {
                ntp_view.update(cx, |settings, cx| settings.set_automatic_time(enabled, cx));
            },
        );

        let now: SharedString = snapshot
            .formatted_local_time_at(current_system_time_usec().unwrap_or(snapshot.time_usec))
            .into();
        let clock_row = if let Some(editor) = &self.clock_editor {
            let review_view = view.clone();
            let cancel_view = view.clone();
            row_base()
                .child(text_block(
                    "Date and time".into(),
                    Some("Use YYYY-MM-DD HH:MM:SS ±HH:MM".into()),
                ))
                .child(div().w(px(220.0)).child(TextField::new(editor).small()))
                .child(
                    push_button("clock-edit-cancel", "Cancel")
                        .disabled(self.time_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| settings.cancel_clock_edit(cx));
                        }),
                )
                .child(
                    Button::new("clock-edit-review", "Review…")
                        .primary()
                        .disabled(self.time_busy)
                        .on_click(move |_, _, cx| {
                            review_view
                                .update(cx, |settings, cx| settings.prepare_clock_change(cx));
                        }),
                )
                .into_any_element()
        } else {
            let edit_view = view.clone();
            // The Mac offers Set… only while automatic time is off.
            let set = (!snapshot.ntp_enabled).then(|| {
                push_button("clock-edit", "Set…")
                    .disabled(self.time_busy || self.timezone_editor.is_some())
                    .on_click(move |_, window, cx| {
                        edit_view.update(cx, |settings, cx| settings.start_clock_edit(window, cx));
                    })
                    .into_any_element()
            });
            value_button_row("Date and time", None, Some(now), set)
        };

        let timezone_row = if let Some(editor) = &self.timezone_editor {
            let save_view = view.clone();
            let cancel_view = view.clone();
            row_base()
                .child(text_block(
                    "Time zone".into(),
                    Some("Enter an installed zone such as Asia/Kolkata".into()),
                ))
                .child(div().w(px(180.0)).child(TextField::new(editor).small()))
                .child(
                    push_button("timezone-cancel", "Cancel")
                        .disabled(self.time_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view
                                .update(cx, |settings, cx| settings.cancel_timezone_edit(cx));
                        }),
                )
                .child(
                    Button::new("timezone-save", "Save")
                        .primary()
                        .busy(self.time_busy)
                        .disabled(self.time_busy)
                        .on_click(move |_, _, cx| {
                            save_view.update(cx, |settings, cx| settings.submit_timezone(cx));
                        }),
                )
                .into_any_element()
        } else {
            let edit_view = view.clone();
            value_button_row(
                "Time zone",
                None,
                Some(snapshot.timezone.clone().into()),
                Some(
                    push_button("timezone-edit", "Set…")
                        .disabled(
                            self.time_busy
                                || self.clock_editor.is_some()
                                || self.clock_confirmation.is_some(),
                        )
                        .on_click(move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_timezone_edit(window, cx)
                            });
                        })
                        .into_any_element(),
                ),
            )
        };

        let mut cards = vec![
            card(vec![automatic]),
            card(vec![clock_row]),
            card(vec![timezone_row]),
        ];
        if snapshot.timezones_truncated {
            cards.push(footnote(
                "The installed time-zone list was too long to check completely.",
            ));
        }
        cards.push(footer_buttons(vec![refresh]));
        self.pane(cards)
    }
}
