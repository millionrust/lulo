//! Date & Time snapshots, timezone mutation, and reviewed manual-clock lifecycle.

use super::*;

impl Settings {
    pub(super) fn finish_time_update(
        &mut self,
        result: std::result::Result<rmac_time::Snapshot, rmac_time::Error>,
    ) {
        self.time_loading = false;
        self.time_busy = false;
        match result {
            Ok(snapshot) => {
                if snapshot.ntp_enabled {
                    self.clock_editor = None;
                    self.clock_confirmation = None;
                }
                self.time = Some(snapshot);
                self.time_error = None;
                self.time_stream_error = None;
            }
            Err(error) => {
                self.time_error = Some(format!("Could not update date and time: {error}").into());
            }
        }
    }

    pub(super) fn queue_time_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.time_loading || self.time_busy || self.time_stream_refreshing {
            self.time_refresh_pending = true;
            return;
        }
        self.time_refresh_pending = false;
        self.time_stream_refreshing = true;
        let generation = self.time_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_time_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.time_stream_refreshing = false;
                if time_stream_snapshot_is_current(
                    generation,
                    this.time_generation,
                    this.time_loading,
                    this.time_busy,
                ) {
                    match result {
                        Ok(snapshot) => {
                            if snapshot.ntp_enabled {
                                this.clock_editor = None;
                                this.clock_confirmation = None;
                            }
                            this.time = Some(snapshot);
                            this.time_error = None;
                            this.time_stream_error = None;
                        }
                        Err(_) => {
                            this.time_stream_error =
                                Some("Could not refresh the changed date and time state".into());
                        }
                    }
                } else {
                    this.time_refresh_pending = true;
                }
                this.run_pending_time_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_pending_time_refresh(&mut self, cx: &mut Context<Self>) {
        if self.time_refresh_pending
            && !self.time_loading
            && !self.time_busy
            && !self.time_stream_refreshing
        {
            self.queue_time_stream_refresh(cx);
        }
    }

    pub(super) fn refresh_time(&mut self, cx: &mut Context<Self>) {
        if self.time_loading || self.time_busy {
            return;
        }
        self.time_busy = true;
        self.time_generation = self.time_generation.wrapping_add(1);
        self.time_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_time_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_time_update(result);
                this.run_pending_time_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_automatic_time(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.time_loading || self.time_busy {
            return;
        }
        self.time_busy = true;
        self.time_generation = self.time_generation.wrapping_add(1);
        self.time_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_time_linux::set_ntp(enabled) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_time_update(result);
                this.run_pending_time_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn start_timezone_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.time_busy
            || self.timezone_editor.is_some()
            || self.clock_editor.is_some()
            || self.clock_confirmation.is_some()
        {
            return;
        }
        let Some(snapshot) = &self.time else {
            return;
        };
        let timezone = snapshot.timezone.clone();
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(timezone)
                .placeholder("Asia/Kolkata")
        });
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        self.timezone_editor = Some(editor);
        self.time_error = None;
        cx.notify();
    }

    pub(super) fn cancel_timezone_edit(&mut self, cx: &mut Context<Self>) {
        if !self.time_busy {
            self.timezone_editor = None;
            self.time_error = None;
            cx.notify();
        }
    }

    pub(super) fn submit_timezone(&mut self, cx: &mut Context<Self>) {
        if self.time_busy {
            return;
        }
        let (Some(editor), Some(snapshot)) = (&self.timezone_editor, &self.time) else {
            return;
        };
        let timezone = editor.read(cx).value().trim().to_string();
        if let Err(error) = snapshot.validate_timezone(&timezone) {
            self.time_error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        self.time_busy = true;
        self.time_generation = self.time_generation.wrapping_add(1);
        self.time_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_time_linux::set_timezone(&timezone) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if result.is_ok() {
                    this.timezone_editor = None;
                }
                this.finish_time_update(result);
                this.run_pending_time_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn start_clock_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.time_busy
            || self.clock_editor.is_some()
            || self.timezone_editor.is_some()
            || self.clock_confirmation.is_some()
        {
            return;
        }
        let Some(snapshot) = &self.time else {
            return;
        };
        if snapshot.ntp_enabled {
            self.time_error =
                Some("Turn off automatic time before setting the clock manually".into());
            cx.notify();
            return;
        }
        let current = current_system_time_usec().unwrap_or(snapshot.time_usec);
        let Some(value) = snapshot.clock_input_at(current) else {
            self.time_error = Some("The current system time could not be edited safely".into());
            cx.notify();
            return;
        };
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value)
                .placeholder("2026-07-18 11:30:00 +05:30")
        });
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        self.clock_editor = Some(editor);
        self.clock_confirmation = None;
        self.time_error = None;
        cx.notify();
    }

    pub(super) fn cancel_clock_edit(&mut self, cx: &mut Context<Self>) {
        if !self.time_busy {
            self.clock_editor = None;
            self.clock_confirmation = None;
            self.time_error = None;
            cx.notify();
        }
    }

    pub(super) fn cancel_clock_confirmation(&mut self, cx: &mut Context<Self>) {
        if !self.time_busy {
            self.clock_confirmation = None;
            cx.notify();
        }
    }

    pub(super) fn prepare_clock_change(&mut self, cx: &mut Context<Self>) {
        if self.time_busy {
            return;
        }
        let (Some(editor), Some(snapshot)) = (&self.clock_editor, &self.time) else {
            return;
        };
        if snapshot.ntp_enabled {
            self.time_error =
                Some("Turn off automatic time before setting the clock manually".into());
            cx.notify();
            return;
        }
        match rmac_time::ClockTarget::parse(&editor.read(cx).value()) {
            Ok(target) => {
                self.clock_confirmation = Some(target);
                self.time_error = None;
            }
            Err(error) => {
                self.time_error = Some(error.to_string().into());
            }
        }
        cx.notify();
    }

    pub(super) fn confirm_clock_change(&mut self, cx: &mut Context<Self>) {
        if self.time_busy {
            return;
        }
        let Some(target) = self.clock_confirmation.clone() else {
            return;
        };
        self.time_busy = true;
        self.clock_setting = true;
        self.time_generation = self.time_generation.wrapping_add(1);
        self.time_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_time_linux::set_time(&target) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.clock_setting = false;
                this.clock_confirmation = None;
                if result.is_ok() {
                    this.clock_editor = None;
                }
                this.finish_time_update(result);
                this.time_refresh_pending = true;
                this.run_pending_time_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_clock_confirmation(&self, cx: &Context<Self>) -> Option<AnyElement> {
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
    pub(super) fn render_date_time(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = Button::new("refresh-date-time", "Refresh")
            .busy(self.time_busy || self.time_stream_refreshing)
            .disabled(self.time_loading || self.time_busy || self.time_stream_refreshing)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_time(cx));
            });
        let Some(snapshot) = &self.time else {
            return self.pane(vec![
                card(vec![row_base()
                    .child(tile("icons/clock.svg", secondary(), 22.0))
                    .child(text_block(
                        "System date and time".into(),
                        Some("systemd-timedated".into()),
                    ))
                    .child(refresh)
                    .into_any_element()]),
                note_card(if self.time_loading {
                    "Reading authoritative date and time state from systemd-timedated…"
                } else {
                    "The system date and time service is unavailable. No local fallback controls are shown."
                }),
            ]);
        };

        let timezone_row = if let Some(editor) = &self.timezone_editor {
            let save_view = view.clone();
            let cancel_view = view.clone();
            row_base()
                .child(tile("icons/globe.svg", accent(), 22.0))
                .child(text_block(
                    "Time zone".into(),
                    Some("Enter an exact system zone such as Asia/Kolkata".into()),
                ))
                .child(div().w(px(180.0)).child(TextField::new(editor).small()))
                .child(
                    Button::new("timezone-cancel", "Cancel")
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
            row_base()
                .child(tile("icons/globe.svg", accent(), 22.0))
                .child(text_block(
                    "Time zone".into(),
                    Some("Validated against zones installed on this system".into()),
                ))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child(snapshot.timezone.clone()),
                )
                .child(
                    Button::new("timezone-edit", "Edit")
                        .disabled(
                            self.time_busy
                                || self.clock_editor.is_some()
                                || self.clock_confirmation.is_some(),
                        )
                        .on_click(move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_timezone_edit(window, cx)
                            });
                        }),
                )
                .into_any_element()
        };

        let clock_row = if let Some(editor) = &self.clock_editor {
            let review_view = view.clone();
            let cancel_view = view.clone();
            row_base()
                .child(tile("icons/clock.svg", accent(), 22.0))
                .child(text_block(
                    "Set date and time".into(),
                    Some("Use YYYY-MM-DD HH:MM:SS ±HH:MM".into()),
                ))
                .child(div().w(px(250.0)).child(TextField::new(editor).small()))
                .child(
                    Button::new("clock-edit-cancel", "Cancel")
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
            row_base()
                .child(tile("icons/clock.svg", accent(), 22.0))
                .child(text_block(
                    "Set date and time".into(),
                    Some(
                        if snapshot.ntp_enabled {
                            "Turn off automatic time to edit the system clock"
                        } else {
                            "Authorization may be required"
                        }
                        .into(),
                    ),
                ))
                .child(
                    Button::new("clock-edit", "Edit")
                        .disabled(
                            self.time_busy
                                || snapshot.ntp_enabled
                                || self.timezone_editor.is_some(),
                        )
                        .on_click(move |_, window, cx| {
                            edit_view
                                .update(cx, |settings, cx| settings.start_clock_edit(window, cx));
                        }),
                )
                .into_any_element()
        };

        let ntp_view = view.clone();
        let automatic = Toggle::new("automatic-time")
            .checked(snapshot.ntp_enabled)
            .disabled(self.time_busy || !snapshot.can_ntp)
            .on_click(move |enabled, _, cx| {
                ntp_view.update(cx, |settings, cx| settings.set_automatic_time(*enabled, cx));
            });
        let synchronization = if !snapshot.can_ntp {
            "No synchronization service"
        } else if snapshot.synchronized {
            "Synchronized"
        } else if snapshot.ntp_enabled {
            "Synchronizing"
        } else {
            "Off"
        };
        let mut cards = vec![
            card(vec![
                value_row(
                    "icons/clock.svg",
                    secondary(),
                    "Current time".into(),
                    snapshot
                        .formatted_local_time_at(
                            current_system_time_usec().unwrap_or(snapshot.time_usec),
                        )
                        .into(),
                ),
                value_row(
                    "icons/refresh-cw.svg",
                    if snapshot.synchronized {
                        hsl(0x34c759)
                    } else {
                        secondary()
                    },
                    "Synchronization".into(),
                    synchronization.into(),
                ),
                row_base()
                    .child(tile("icons/refresh-cw.svg", accent(), 22.0))
                    .child(text_block(
                        "Set time automatically".into(),
                        Some(if snapshot.can_ntp {
                            "Use the system network time service".into()
                        } else {
                            "No compatible network time service is installed".into()
                        }),
                    ))
                    .child(automatic)
                    .into_any_element(),
                clock_row,
            ]),
            card(vec![timezone_row]),
            card(vec![
                value_row(
                    "icons/settings.svg",
                    secondary(),
                    "Hardware clock".into(),
                    if snapshot.local_rtc {
                        "Local time"
                    } else {
                        "UTC"
                    }
                    .into(),
                ),
                row_base()
                    .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
                    .child(text_block(
                        "Authoritative state".into(),
                        Some("Live timedated changes · refresh on demand".into()),
                    ))
                    .child(refresh)
                    .into_any_element(),
            ]),
        ];
        if snapshot.timezones_truncated {
            cards.push(note_card(
                "The installed time-zone inventory exceeded the bounded validation list.",
            ));
        }
        cards.push(note_card(
            "Manual input includes a numeric UTC offset so daylight-saving transitions are never guessed. The hardware-clock mode remains read-only; UTC is the recommended Linux configuration.",
        ));
        self.pane(cards)
    }
}
