//! Spotlight settings presentation, laid out like macOS 26: the header card,
//! the recent-documents action, "Results from System", the file privacy
//! switches and the excluded-folder list (design-lab/settings.html).

use super::*;

impl Settings {
    pub(in crate::controller) fn render_spotlight(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let mut cards = vec![header_card(
            tile26("icons/search.svg", accent()),
            "Spotlight",
            "Spotlight helps you quickly find applications, settings, files and calculations on your computer.",
            None,
        )];

        let clear_history_view = view.clone();
        cards.push(card(vec![value_button_row(
            "Recent Documents",
            Some("Documents that Files and Spotlight list as recent. Clearing never deletes a document.".into()),
            None,
            Some(
                push_button(
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
                })
                .into_any_element(),
            ),
        )]));
        if let Some(notice) = &self.recent_history_notice {
            cards.push(footnote(notice.clone()));
        }
        if self.recent_history_confirmation {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(footnote(
                "Clear recent documents from Spotlight? Files are not deleted. Other applications keep managing their own history.",
            ));
            cards.push(footer_buttons(vec![
                push_button("spotlight-clear-history-cancel", "Cancel")
                    .on_click(move |_, _, cx| {
                        cancel_view
                            .update(cx, |settings, cx| settings.cancel_recent_history_clear(cx));
                    })
                    .into_any_element(),
                rmac_ui::dialog_button(
                    "spotlight-clear-history-confirm",
                    "Clear History",
                    rmac_ui::DialogButtonKind::Destructive,
                )
                .disabled(self.recent_history_busy)
                .on_click(move |_, _, cx| {
                    confirm_view
                        .update(cx, |settings, cx| settings.confirm_recent_history_clear(cx));
                })
                .into_any_element(),
            ]));
        }

        if self.shell_settings_loading && self.shell_settings.is_none() {
            cards.push(footnote("Loading search preferences…"));
        } else if let Some(snapshot) = self.shell_settings.as_ref() {
            self.append_spotlight_settings(view.clone(), &snapshot.settings, &mut cards);
        } else {
            cards.push(note_card(
                "The Lulo OS shell-settings service is unavailable. Search preferences remain unchanged.",
            ));
        }

        if let Some(error) = self.shortcut_status_error.clone() {
            cards.push(footnote(rmac_ui::user_error_message(
                rmac_ui::ErrorSurface::Settings,
                error.as_ref(),
                false,
            )));
        }
        if let Some(error) = self.shortcut_configuration_error.clone() {
            cards.push(footnote(rmac_ui::user_error_message(
                rmac_ui::ErrorSurface::Settings,
                error.as_ref(),
                false,
            )));
        }

        let shortcuts_view = view.clone();
        let revert_view = view.clone();
        let refresh_view = view;
        let mut buttons = vec![
            push_button("spotlight-keyboard-shortcuts", "Keyboard Shortcuts…")
                .on_click(move |_, _, cx| {
                    shortcuts_view.update(cx, |settings, cx| {
                        settings.keyboard_shortcuts_open = true;
                        cx.notify();
                    });
                })
                .into_any_element(),
        ];
        if self.spotlight_revert.is_some() {
            buttons.push(
                push_button("spotlight-revert", "Revert")
                    .disabled(self.shell_settings_loading || self.shell_settings_busy)
                    .on_click(move |_, _, cx| {
                        revert_view.update(cx, |settings, cx| settings.revert_spotlight_change(cx));
                    })
                    .into_any_element(),
            );
        }
        buttons.push(
            push_button(
                "spotlight-refresh",
                if self.shell_settings_busy {
                    "Applying…"
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
            })
            .into_any_element(),
        );
        cards.push(footer_buttons(buttons));
        self.pane(cards)
    }

    fn append_spotlight_settings(
        &self,
        view: Entity<Self>,
        settings: &rmac_shell_settings::ShellSettings,
        cards: &mut Vec<Div>,
    ) {
        let enabled = !self.shell_settings_busy;
        let policy = |id| spotlight_provider_policy(settings, id);
        let files = policy(rmac_launcher_providers::FILES_PROVIDER);
        let currency = policy(rmac_launcher_providers::CURRENCY_PROVIDER);

        // "Results from System": a heading inside the group, then a 42 pt
        // row with a switch per provider.
        let mut results = vec![group_heading(
            "Results from System",
            Some("Allows items and their content to appear in Spotlight.".into()),
        )
        .into_any_element()];
        for (id, icon, color, title) in [
            (
                rmac_launcher_providers::APPLICATIONS_PROVIDER,
                "icons/app-window.svg",
                accent(),
                "Apps",
            ),
            (
                rmac_launcher_providers::SETTINGS_PROVIDER,
                "icons/settings.svg",
                hsl(0x8e8e93),
                "System Settings",
            ),
            (
                rmac_launcher_providers::FILES_PROVIDER,
                "icons/folder-symlink.svg",
                hsl(0x30b0c7),
                "Files",
            ),
            (
                rmac_launcher_providers::CALCULATOR_PROVIDER,
                "icons/database.svg",
                hsl(0x8e8e93),
                "Calculator",
            ),
        ] {
            results.push(spotlight_provider_row(
                view.clone(),
                id,
                icon,
                color,
                title,
                policy(id).enabled,
                enabled,
            ));
        }
        cards.push(card(results));
        if files.enabled && !files.allow_private_content {
            cards.push(footnote(
                "Files shows no results until private file results are allowed.",
            ));
        }

        let private_view = view.clone();
        let removable_view = view.clone();
        let currency_view = view.clone();
        cards.push(card(vec![
            switch_row(
                "spotlight-private-files",
                "Allow private file results",
                Some("Show local file names and recent documents in results.".into()),
                files.allow_private_content,
                enabled,
                move |value, _, cx| {
                    private_view.update(cx, |settings, cx| {
                        settings.apply_spotlight_change(
                            SpotlightChange::ProviderPrivateContent {
                                id: rmac_launcher_providers::FILES_PROVIDER.into(),
                                allowed: value,
                            },
                            cx,
                        )
                    });
                },
            ),
            switch_row(
                "spotlight-removable-mounts",
                "Include removable volumes",
                Some("Search disks and volumes beyond the home folder.".into()),
                settings.spotlight.include_removable_mounts,
                enabled,
                move |value, _, cx| {
                    removable_view.update(cx, |settings, cx| {
                        settings.apply_spotlight_change(
                            SpotlightChange::IncludeRemovableMounts(value),
                            cx,
                        )
                    });
                },
            ),
            // Currency answers need the European Central Bank's daily
            // rates: the only Spotlight feature that uses the network, so
            // it waits for permission. Queries never leave the computer.
            switch_row(
                "spotlight-currency-rates",
                "Currency conversions",
                Some(
                    "Download daily exchange rates from the European Central Bank. \
                     Searches stay on this computer."
                        .into(),
                ),
                currency.enabled && currency.allow_network,
                enabled,
                move |value, _, cx| {
                    currency_view.update(cx, |settings, cx| {
                        settings.apply_spotlight_change(
                            SpotlightChange::ProviderNetwork {
                                id: rmac_launcher_providers::CURRENCY_PROVIDER.into(),
                                allowed: value,
                            },
                            cx,
                        )
                    });
                },
            ),
        ]));

        // "Search Privacy": the excluded folders as a table with +/−. Each
        // row carries its own Remove, since the well keeps no selection.
        cards.push(section_with_note(
            "Search Privacy",
            "Prevent Spotlight from searching these locations.",
            false,
        ));
        let rows = settings
            .spotlight
            .excluded_paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let remove_view = view.clone();
                let remove_path = path.clone();
                well_row(
                    SharedString::from(format!("spotlight-exclusion-{index}")),
                    false,
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(glyph("icons/folder-symlink.svg", 14.0, secondary()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(rmac_ui::text_px(13.0))
                                .text_color(label())
                                .child(path.clone()),
                        )
                        .child(
                            push_button(
                                SharedString::from(format!("spotlight-remove-exclusion-{index}")),
                                "Remove",
                            )
                            .h(px(20.0))
                            .disabled(!enabled)
                            .on_click(move |_, _, cx| {
                                remove_view.update(cx, |settings, cx| {
                                    settings.apply_spotlight_change(
                                        SpotlightChange::RemoveExclusion(remove_path.clone()),
                                        cx,
                                    )
                                });
                            }),
                        ),
                    None,
                )
            })
            .collect::<Vec<_>>();
        let add: Option<FormHandler> = enabled.then(|| {
            let choose_view = view.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                choose_view.update(cx, |settings, cx| settings.choose_search_exclusion(cx));
            }) as FormHandler
        });
        cards.push(well(None, rows, 3, add, None));
    }
}
