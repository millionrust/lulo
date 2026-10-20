//! General ▸ Startup Disk: pick the boot entry the next start uses, then
//! restart, as on macOS (`crate::startup_disk`).

use super::*;

impl Settings {
    /// Read the boot entries off the UI thread when the page opens.
    pub(super) fn load_startup_disks(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let disks = cx
                .background_executor()
                .spawn(async { crate::startup_disk::read() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.startup_disk_selected = disks.current;
                this.startup_disks = Some(disks);
                cx.notify();
            });
        })
        .detach();
    }

    fn select_startup_disk(&mut self, number: u16, cx: &mut Context<Self>) {
        self.startup_disk_selected = Some(number);
        self.startup_disk_error = None;
        cx.notify();
    }

    fn confirm_startup_disk_restart(&mut self, open: bool, cx: &mut Context<Self>) {
        self.startup_disk_confirming = open;
        cx.notify();
    }

    /// Set `BootNext` (polkit asks for the password), then restart through
    /// the menu bar so every app gets to save first.
    fn restart_to_startup_disk(&mut self, cx: &mut Context<Self>) {
        self.startup_disk_confirming = false;
        let Some(number) = self.startup_disk_selected else {
            return;
        };
        self.startup_disk_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // pkexec is a child process: keep it off GPUI's executor.
            let result = blocking::unblock(move || crate::startup_disk::set_next(number)).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.startup_disk_busy = false;
                match result {
                    Ok(()) => {
                        let id = rmac_shortcuts::ShortcutId(
                            rmac_shortcuts::power_key::RESTART_TO_UPDATE_SHORTCUT.into(),
                        );
                        if let Err(error) = rmac_shortcuts::dispatch(&id) {
                            this.startup_disk_error = Some(
                                format!("The startup disk is set, but Settings could not restart: {error}. Restart from the Lulo menu.")
                                    .into(),
                            );
                        }
                    }
                    Err(error) => {
                        this.startup_disk_error = Some(format!("Could not change the startup disk: {error}.").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn startup_disk_body(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let Some(disks) = &self.startup_disks else {
            return div().child(note_card("Reading the startup disks…"));
        };
        let mut cards = Vec::new();
        if disks.entries.is_empty() {
            cards.push(note_card("The firmware reports no startup disks."));
            return div().v_flex().children(cards);
        }
        let selected = self.startup_disk_selected;
        let rows = disks
            .entries
            .iter()
            .map(|entry| {
                let number = entry.number;
                let is_selected = selected == Some(number);
                let current = disks.current == Some(number);
                let select_view = view.clone();
                div()
                    .id(SharedString::from(format!("startup-disk-{number:04X}")))
                    .role(Role::RadioButton)
                    .aria_label(SharedString::from(entry.description.clone()))
                    .aria_selected(is_selected)
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        select_view
                            .update(cx, |settings, cx| settings.select_startup_disk(number, cx));
                    })
                    .child(
                        row_base()
                            .when(is_selected, |row| row.bg(style::sidebar_selection()))
                            .child(tile(
                                "icons/hard-drive.svg",
                                if is_selected { accent() } else { secondary() },
                                style::HEADER_ICON,
                            ))
                            .child(text_block(
                                entry.description.clone().into(),
                                current.then(|| "Current startup disk".into()),
                            )),
                    )
                    .into_any_element()
            })
            .collect();
        cards.push(card(rows));
        if let Some(entry) = disks
            .entries
            .iter()
            .find(|entry| Some(entry.number) == selected)
        {
            cards.push(footnote(format!(
                "You have selected “{}” as the startup disk.",
                entry.description
            )));
        }
        let restart_view = view.clone();
        cards.push(footer_buttons(vec![push_button(
            "startup-disk-restart",
            "Restart…",
        )
        .busy(self.startup_disk_busy)
        .disabled(self.startup_disk_busy || selected.is_none())
        .on_click(move |_, _, cx| {
            restart_view.update(cx, |settings, cx| {
                settings.confirm_startup_disk_restart(true, cx)
            });
        })
        .into_any_element()]));
        if let Some(error) = &self.startup_disk_error {
            cards.push(note_card(error.clone()));
        }
        div().v_flex().children(cards)
    }

    /// "Are you sure you want to restart?" over the page.
    pub(super) fn render_startup_disk_alert(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.startup_disk_confirming {
            return None;
        }
        let name = self
            .startup_disks
            .as_ref()
            .and_then(|disks| {
                disks
                    .entries
                    .iter()
                    .find(|entry| Some(entry.number) == self.startup_disk_selected)
            })
            .map(|entry| entry.description.clone())
            .unwrap_or_default();
        Some(
            rmac_ui::alert(
                "Are you sure you want to restart the computer?",
                format!("Your computer will start up using “{name}”. Apps are asked to quit first, so you can save your work."),
                vec![
                    rmac_ui::dialog_button(
                        "startup-disk-cancel",
                        "Cancel",
                        rmac_ui::DialogButtonKind::Normal,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.confirm_startup_disk_restart(false, cx)))
                    .into_any_element(),
                    rmac_ui::dialog_button(
                        "startup-disk-confirm",
                        "Restart",
                        rmac_ui::DialogButtonKind::Primary,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.restart_to_startup_disk(cx)))
                    .into_any_element(),
                ],
            )
            .into_any_element(),
        )
    }
}
