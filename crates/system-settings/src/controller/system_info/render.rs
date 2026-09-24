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
        let mut cards = cards;
        // The first-login Setup Assistant, run again on request; offered
        // only where it is installed.
        if let Some(program) = setup_assistant_program() {
            cards.push(footer_buttons(vec![push_button(
                "general-setup-assistant",
                "Setup Assistant…",
            )
            .on_click(move |_, _, _| {
                if let Err(error) = std::process::Command::new(program)
                    .stdin(std::process::Stdio::null())
                    .spawn()
                {
                    eprintln!("could not open Setup Assistant: {error}");
                }
            })
            .into_any_element()]));
        }
        self.pane(cards)
    }
    /// macOS 26 About: the machine centred over its name, then the Name,
    /// chip and memory group, the operating system and the startup volume.
    pub(in crate::controller) fn about_body(&self, cx: &Context<Self>) -> Div {
        let si = &self.sysinfo;
        let view = cx.entity();
        let hostname_row = if let Some(editor) = &self.hostname_editor {
            let save_view = view.clone();
            let cancel_view = view.clone();
            row_base()
                .items_start()
                .child(text_block(
                    "Name".into(),
                    Some("Letters, numbers, and hyphens · 63 bytes maximum".into()),
                ))
                .child(div().w(px(170.0)).child(TextField::new(editor).small()))
                .child(
                    push_button("hostname-cancel", "Cancel")
                        .disabled(self.system_data_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view
                                .update(cx, |settings, cx| settings.cancel_hostname_edit(cx));
                        }),
                )
                .child(
                    Button::new("hostname-save", "Save")
                        .primary()
                        .h(px(24.0))
                        .busy(self.system_data_busy)
                        .disabled(self.system_data_busy)
                        .on_click(move |_, _, cx| {
                            save_view.update(cx, |settings, cx| settings.submit_hostname(cx));
                        }),
                )
                .into_any_element()
        } else {
            let edit_view = view.clone();
            value_button_row(
                "Name",
                si.hostname_unavailable_reason.clone().map(Into::into),
                Some(si.display_hostname().to_owned().into()),
                si.hostname_mutable.then(|| {
                    push_button("hostname-edit", "Edit…")
                        .on_click(move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_hostname_edit(window, cx)
                            });
                        })
                        .into_any_element()
                }),
            )
        };

        let mut facts = vec![hostname_row];
        let chip = si.processor.clone().unwrap_or_else(|| "—".into());
        facts.push(fact_row("Chip", chip));
        facts.push(fact_row(
            "Memory",
            si.memory.clone().unwrap_or_else(|| "—".into()),
        ));
        if let Some(graphics) = &si.graphics {
            facts.push(fact_row("Graphics", graphics.clone()));
        }
        facts.push(fact_row("Architecture", si.architecture.clone()));

        let mut session = Vec::new();
        session.push(
            large_row(tile26("icons/lulo.svg", hsl(0xff8a1e)), "Lulo OS", None)
                .child(trailing_value(format!(
                    "Version {}",
                    env!("CARGO_PKG_VERSION")
                )))
                .into_any_element(),
        );
        session.push(
            large_row(
                tile26("icons/settings.svg", hsl(0x8e8e93)),
                si.operating_system.clone(),
                None,
            )
            .child(trailing_value(si.kernel.clone()))
            .into_any_element(),
        );
        if let Some(desktop) = &si.desktop {
            session.push(fact_row("Desktop", desktop.clone()));
        }
        if let Some(name) = &si.session {
            session.push(fact_row("Session", name.clone()));
        }

        let model = si
            .hardware_model
            .clone()
            .unwrap_or_else(|| si.display_hostname().to_owned());
        let mut body = div()
            .v_flex()
            .child(
                div()
                    .v_flex()
                    .items_center()
                    .pt(px(10.0))
                    .pb(px(20.0))
                    .child(glyph("icons/monitor.svg", 96.0, label()))
                    .child(
                        div()
                            .mt(px(8.0))
                            .text_size(rmac_ui::text_px(26.0))
                            .line_height(px(31.0))
                            .font_weight(rmac_ui::mac::BOLD)
                            .text_color(label())
                            .child(model),
                    )
                    .when_some(si.hardware_vendor.clone(), |header, vendor| {
                        header.child(
                            div()
                                .mt(px(2.0))
                                .text_size(rmac_ui::text_px(11.0))
                                .line_height(px(14.0))
                                .text_color(secondary())
                                .child(vendor),
                        )
                    }),
            )
            .child(card(facts))
            .child(section_header("Operating System"))
            .child(card(session));

        if let Some(volume) = self.home_volume() {
            if let Some(usage) = volume.usage {
                let storage_view = view.clone();
                body = body.child(section_header("Storage")).child(card(vec![
                    large_row(
                        tile26("icons/hard-drive.svg", hsl(0x8e8e93)),
                        volume.mount.name.clone(),
                        None,
                    )
                    .child(trailing_value(format!(
                        "{} available of {}",
                        fmt_gb(usage.available),
                        fmt_gb(usage.total)
                    )))
                    .into_any_element(),
                    button_row(vec![push_button("about-storage", "Storage Settings…")
                        .on_click(move |_, _, cx| {
                            storage_view
                                .update(cx, |settings, cx| settings.push(SubPage::Storage, cx));
                        })
                        .into_any_element()]),
                ]));
            }
        }

        let refresh_view = view.clone();
        let diagnostics_view = view.clone();
        body.child(
            div()
                .flex()
                .justify_center()
                .gap(px(10.0))
                .mt(px(style::GROUP_GAP))
                .child(
                    push_button(
                        "copy-system-report",
                        if self.diagnostics_copied {
                            "Copied"
                        } else {
                            "Copy System Report"
                        },
                    )
                    .on_click(move |_, _, cx| {
                        diagnostics_view.update(cx, |settings, cx| settings.copy_diagnostics(cx));
                    }),
                )
                .child(
                    push_button("refresh-system-information", "Refresh")
                        .busy(self.system_data_busy)
                        .disabled(self.system_data_busy)
                        .on_click(move |_, _, cx| {
                            refresh_view
                                .update(cx, |settings, cx| settings.refresh_system_info(cx));
                        }),
                ),
        )
    }
}

/// The installed Setup Assistant: the packaged copy, else the development
/// install under ~/.local/libexec.
fn setup_assistant_program() -> Option<&'static std::path::Path> {
    static PROGRAM: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    PROGRAM
        .get_or_init(|| {
            let packaged = std::path::PathBuf::from("/usr/libexec/rmac/rmac-setup-assistant");
            let development = std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|home| home.join(".local/libexec/rmac/rmac-setup-assistant"));
            std::iter::once(packaged)
                .chain(development)
                .find(|path| path.is_file())
        })
        .as_deref()
}
