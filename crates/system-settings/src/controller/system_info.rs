//! General/About system snapshot, hostname, accessibility, and diagnostics updates.

use super::*;

impl Settings {
    pub(super) fn apply_system_snapshot(&mut self, snapshot: SystemSnapshot) {
        self.account = snapshot.account.into();
        match snapshot.sysinfo {
            Ok(sysinfo) => {
                self.sysinfo = sysinfo;
                self.system_data_error = None;
            }
            Err(error) => {
                self.system_data_error =
                    Some(format!("Could not read system information: {error}").into());
            }
        }
        match snapshot.storage {
            Ok(storage) => {
                self.storage = storage;
                self.storage_error = None;
                self.storage_stream_error = None;
            }
            Err(error) => {
                self.storage_error =
                    Some(format!("Could not read storage volumes: {error}").into());
            }
        }
        self.system_data_loading = false;
    }

    pub(super) fn queue_system_info_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.system_data_loading || self.system_data_busy || self.system_data_stream_refreshing {
            self.system_data_refresh_pending = true;
            return;
        }
        self.system_data_refresh_pending = false;
        self.system_data_stream_refreshing = true;
        let generation = self.system_data_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_system_info::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.system_data_stream_refreshing = false;
                if system_info_stream_snapshot_is_current(
                    generation,
                    this.system_data_generation,
                    this.system_data_loading,
                    this.system_data_busy,
                ) {
                    match result {
                        Ok(snapshot) => {
                            this.sysinfo = snapshot;
                            this.system_data_error = None;
                            this.system_data_stream_error = None;
                            this.diagnostics_copied = false;
                        }
                        Err(error) => {
                            this.system_data_stream_error = Some(
                                format!("Could not refresh changed system information: {error}")
                                    .into(),
                            );
                        }
                    }
                } else {
                    this.system_data_refresh_pending = true;
                }
                this.run_pending_system_info_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_pending_system_info_refresh(&mut self, cx: &mut Context<Self>) {
        if self.system_data_refresh_pending
            && !self.system_data_loading
            && !self.system_data_busy
            && !self.system_data_stream_refreshing
        {
            self.queue_system_info_stream_refresh(cx);
        }
    }

    pub(super) fn start_hostname_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.system_data_busy || !self.sysinfo.hostname_mutable {
            return;
        }
        let hostname = self
            .sysinfo
            .static_hostname
            .clone()
            .unwrap_or_else(|| self.sysinfo.hostname.clone());
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(hostname)
                .placeholder("studio-pc")
        });
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        self.hostname_editor = Some(editor);
        self.system_data_error = None;
        cx.notify();
    }

    pub(super) fn cancel_hostname_edit(&mut self, cx: &mut Context<Self>) {
        if !self.system_data_busy {
            self.hostname_editor = None;
            self.system_data_error = None;
            cx.notify();
        }
    }

    pub(super) fn submit_hostname(&mut self, cx: &mut Context<Self>) {
        if self.system_data_busy {
            return;
        }
        let Some(editor) = self.hostname_editor.as_ref() else {
            return;
        };
        let hostname = editor.read(cx).value().trim().to_string();
        if let Err(error) = rmac_system_info::validate_static_hostname(&hostname) {
            self.system_data_error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        self.system_data_busy = true;
        self.system_data_generation = self.system_data_generation.wrapping_add(1);
        self.system_data_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_system_info::set_static_hostname(&hostname) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.system_data_busy = false;
                match result {
                    Ok(snapshot) => {
                        this.sysinfo = snapshot;
                        this.hostname_editor = None;
                        this.system_data_error = None;
                        this.system_data_stream_error = None;
                        this.diagnostics_copied = false;
                    }
                    Err(error) => {
                        this.system_data_error = Some(error.to_string().into());
                    }
                }
                this.run_pending_system_info_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn refresh_system_info(&mut self, cx: &mut Context<Self>) {
        if self.system_data_busy {
            return;
        }
        self.system_data_busy = true;
        self.system_data_generation = self.system_data_generation.wrapping_add(1);
        self.system_data_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_system_info::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.system_data_busy = false;
                match result {
                    Ok(snapshot) => {
                        this.sysinfo = snapshot;
                        this.system_data_error = None;
                        this.system_data_stream_error = None;
                        this.diagnostics_copied = false;
                    }
                    Err(error) => {
                        this.system_data_error =
                            Some(format!("Could not refresh system information: {error}").into());
                    }
                }
                this.run_pending_system_info_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn refresh_screen_reader(&mut self, cx: &mut Context<Self>) {
        if self.screen_reader_loading {
            return;
        }
        self.screen_reader_loading = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let capability = cx
                .background_executor()
                .spawn(async { gather_screen_reader_capability() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.screen_reader = capability;
                this.screen_reader_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn copy_diagnostics(&mut self, cx: &mut Context<Self>) {
        let report = self.sysinfo.diagnostic_report();
        cx.write_to_clipboard(ClipboardItem::new_string(report));
        self.diagnostics_copied = true;
        cx.notify();
    }

    pub(super) fn render_general(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let cards = vec![
            card(vec![
                nav_row(
                    view.clone(),
                    "icons/info.svg",
                    hsl(0x8e8e93),
                    GENERAL_DESTINATIONS[0].into(),
                    self.sysinfo.hardware_model.clone().map(Into::into),
                    SubPage::About,
                ),
                nav_row(
                    view.clone(),
                    "icons/refresh-cw.svg",
                    hsl(0x8e8e93),
                    GENERAL_DESTINATIONS[1].into(),
                    Some(self.sysinfo.operating_system.clone().into()),
                    SubPage::SoftwareUpdate,
                ),
                nav_row(
                    view.clone(),
                    "icons/database.svg",
                    hsl(0x8e8e93),
                    GENERAL_DESTINATIONS[2].into(),
                    None,
                    SubPage::Storage,
                ),
            ]),
            note_card(
                "Device continuity and media-receiver controls are hidden until rmac has reviewed Linux service authorities for them.",
            ),
        ];
        self.pane(cards)
    }
    pub(super) fn about_body(&self, cx: &Context<Self>) -> Div {
        let si = &self.sysinfo;
        let view = cx.entity();
        let hostname_row = if let Some(editor) = &self.hostname_editor {
            let save_view = view.clone();
            let cancel_view = view.clone();
            row_base()
                .child(tile("icons/info.svg", secondary(), 22.0))
                .child(text_block(
                    "Hostname".into(),
                    Some("Letters, numbers, and hyphens · 63 bytes maximum".into()),
                ))
                .child(div().w(px(190.0)).child(TextField::new(editor).small()))
                .child(
                    Button::new("hostname-cancel", "Cancel")
                        .disabled(self.system_data_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view
                                .update(cx, |settings, cx| settings.cancel_hostname_edit(cx));
                        }),
                )
                .child(
                    Button::new("hostname-save", "Save")
                        .primary()
                        .busy(self.system_data_busy)
                        .disabled(self.system_data_busy)
                        .on_click(move |_, _, cx| {
                            save_view.update(cx, |settings, cx| settings.submit_hostname(cx));
                        }),
                )
                .into_any_element()
        } else {
            let edit_view = view.clone();
            row_base()
                .child(tile("icons/info.svg", secondary(), 22.0))
                .child(text_block(
                    "Hostname".into(),
                    si.hostname_unavailable_reason.clone().map(Into::into),
                ))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child(si.display_hostname().to_owned()),
                )
                .when(si.hostname_mutable, |row| {
                    row.child(Button::new("hostname-edit", "Edit").on_click(
                        move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_hostname_edit(window, cx)
                            });
                        },
                    ))
                })
                .into_any_element()
        };

        let mut facts = Vec::new();
        if let Some(vendor) = &si.hardware_vendor {
            facts.push(value_row(
                "icons/monitor.svg",
                secondary(),
                "Manufacturer".into(),
                vendor.clone().into(),
            ));
        }
        facts.extend([
            value_row(
                "icons/monitor.svg",
                secondary(),
                "Model".into(),
                si.hardware_model
                    .clone()
                    .unwrap_or_else(|| "—".into())
                    .into(),
            ),
            value_row(
                "icons/settings.svg",
                secondary(),
                "Processor".into(),
                si.processor.clone().unwrap_or_else(|| "—".into()).into(),
            ),
            value_row(
                "icons/database.svg",
                secondary(),
                "Memory".into(),
                si.memory.clone().unwrap_or_else(|| "—".into()).into(),
            ),
            value_row(
                "icons/refresh-cw.svg",
                secondary(),
                "Operating System".into(),
                si.operating_system.clone().into(),
            ),
            value_row(
                "icons/info.svg",
                secondary(),
                "Kernel".into(),
                si.kernel.clone().into(),
            ),
            value_row(
                "icons/settings.svg",
                secondary(),
                "Architecture".into(),
                si.architecture.clone().into(),
            ),
        ]);
        if let Some(graphics) = &si.graphics {
            facts.push(value_row(
                "icons/monitor.svg",
                secondary(),
                "Graphics".into(),
                graphics.clone().into(),
            ));
        }
        if let Some(session) = &si.session {
            facts.push(value_row(
                "icons/panel-top.svg",
                secondary(),
                "Session".into(),
                session.clone().into(),
            ));
        }
        if let Some(desktop) = &si.desktop {
            facts.push(value_row(
                "icons/panel-top.svg",
                secondary(),
                "Desktop".into(),
                desktop.clone().into(),
            ));
        }

        let refresh_view = view.clone();
        let diagnostics_view = view.clone();
        let diagnostics = card(vec![
            row_base()
                .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
                .child(text_block(
                    "System information".into(),
                    Some("Refresh facts changed outside rmac".into()),
                ))
                .child(
                    Button::new("refresh-system-information", "Refresh")
                        .busy(self.system_data_busy)
                        .disabled(self.system_data_busy)
                        .on_click(move |_, _, cx| {
                            refresh_view
                                .update(cx, |settings, cx| settings.refresh_system_info(cx));
                        }),
                )
                .into_any_element(),
            row_base()
                .child(tile("icons/info.svg", accent(), 22.0))
                .child(text_block(
                    "System report".into(),
                    Some(
                        "Excludes hostname, username, serial numbers, addresses, and paths".into(),
                    ),
                ))
                .child(
                    Button::new(
                        "copy-system-report",
                        if self.diagnostics_copied {
                            "Copied"
                        } else {
                            "Copy"
                        },
                    )
                    .on_click(move |_, _, cx| {
                        diagnostics_view.update(cx, |settings, cx| settings.copy_diagnostics(cx));
                    }),
                )
                .into_any_element(),
        ]);

        div()
            .v_flex()
            .child(card(vec![hostname_row]))
            .child(card(facts))
            .child(diagnostics)
    }
}
