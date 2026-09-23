//! General and About settings presentation.

use super::*;

impl Settings {
    pub(in crate::controller) fn render_general(&self, cx: &Context<Self>) -> Div {
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
            // macOS 26 files these panes under General rather than in the
            // sidebar; each row opens the pane itself.
            card(vec![
                pane_nav_row(view.clone(), "icons/clock.svg", accent(), "Date & Time"),
                pane_nav_row(
                    view.clone(),
                    "icons/languages.svg",
                    accent(),
                    "Language & Region",
                ),
                pane_nav_row(
                    view.clone(),
                    "icons/app-window.svg",
                    hsl(0x8e8e93),
                    "Login Items",
                ),
                pane_nav_row(view, "icons/globe.svg", hsl(0x8e8e93), "Sharing"),
            ]),
        ];
        self.pane(cards)
    }
    pub(in crate::controller) fn about_body(&self, cx: &Context<Self>) -> Div {
        let si = &self.sysinfo;
        let view = cx.entity();
        let hostname_row = if let Some(editor) = &self.hostname_editor {
            let save_view = view.clone();
            let cancel_view = view.clone();
            row_base()
                .child(tile("icons/info.svg", secondary(), style::ROW_ICON))
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
                .child(tile("icons/info.svg", secondary(), style::ROW_ICON))
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
                .child(tile("icons/refresh-cw.svg", secondary(), style::ROW_ICON))
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
                .child(tile("icons/info.svg", accent(), style::ROW_ICON))
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
