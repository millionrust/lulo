//! Spotlight policy, recent-history, shortcut, and pane authority.

use super::*;

impl Settings {
    pub(super) fn apply_spotlight_change(
        &mut self,
        change: SpotlightChange,
        cx: &mut Context<Self>,
    ) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            return;
        };
        let previous = SpotlightAuthority::from_settings(&snapshot.settings);
        let mut next = snapshot.settings.clone();
        change.clone().apply(&mut next);
        if SpotlightAuthority::from_settings(&next) == previous {
            return;
        }

        self.shell_settings_busy = true;
        self.spotlight_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::Spotlight(change))
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.finish_spotlight_mutation(result, Some(previous)) {
                    this.refresh_wallpaper_preview(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn finish_spotlight_mutation(
        &mut self,
        result: std::result::Result<rmac_shell_settings::Snapshot, rmac_shell_settings::Error>,
        previous: Option<SpotlightAuthority>,
    ) -> bool {
        self.shell_settings_loading = false;
        self.shell_settings_busy = false;
        match result {
            Ok(snapshot) => {
                let wallpaper_changed = self.shell_settings.as_ref().is_none_or(|current| {
                    current.settings.wallpaper != snapshot.settings.wallpaper
                });
                if self
                    .shell_settings
                    .as_ref()
                    .is_some_and(|current| current.settings.dock != snapshot.settings.dock)
                {
                    self.shell_settings_revert = None;
                }
                if wallpaper_changed {
                    self.wallpaper_revert = None;
                }
                self.shell_settings = Some(snapshot);
                self.spotlight_revert = previous;
                self.spotlight_error = None;
                self.shell_settings_error = None;
                self.shell_settings_stream_error = None;
                wallpaper_changed
            }
            Err(error) => {
                self.spotlight_error = Some(format!("Could not update Spotlight: {error}").into());
                false
            }
        }
    }

    pub(super) fn choose_search_exclusion(&mut self, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        self.shell_settings_busy = true;
        self.spotlight_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let choice = rmac_portal::choose_search_exclusion().await;
            let validated = match choice {
                Ok(Some(path)) => {
                    Some(blocking::unblock(move || validate_search_exclusion(path)).await)
                }
                Ok(None) => None,
                Err(error) => Some(Err(error.to_string())),
            };
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.shell_settings_busy = false;
                match validated {
                    Some(Ok(path)) => {
                        this.apply_spotlight_change(SpotlightChange::AddExclusion(path), cx)
                    }
                    Some(Err(error)) => this.spotlight_error = Some(error.into()),
                    None => this.refresh_shell_settings(false, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn revert_spotlight_change(&mut self, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(previous) = self.spotlight_revert.clone() else {
            return;
        };
        self.shell_settings_busy = true;
        self.spotlight_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::RestoreSpotlight(previous))
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.finish_spotlight_mutation(result, None) {
                    this.refresh_wallpaper_preview(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn request_recent_history_clear(&mut self, cx: &mut Context<Self>) {
        if self.recent_history_busy {
            return;
        }
        self.recent_history_confirmation = true;
        self.recent_history_notice = None;
        self.spotlight_error = None;
        cx.notify();
    }

    pub(super) fn cancel_recent_history_clear(&mut self, cx: &mut Context<Self>) {
        if !self.recent_history_busy && self.recent_history_confirmation {
            self.recent_history_confirmation = false;
            cx.notify();
        }
    }

    pub(super) fn confirm_recent_history_clear(&mut self, cx: &mut Context<Self>) {
        if self.recent_history_busy || !self.recent_history_confirmation {
            return;
        }
        self.recent_history_confirmation = false;
        self.recent_history_busy = true;
        self.recent_history_notice = None;
        self.spotlight_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                rmac_recent_documents::Store::from_environment().and_then(|store| store.clear())
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.recent_history_busy = false;
                match result {
                    Ok(_) => {
                        this.recent_history_notice =
                            Some("Recent document history was cleared for rmac Search.".into());
                    }
                    Err(_) => {
                        this.spotlight_error =
                            Some("Could not clear recent document history.".into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn finish_shortcut_status_update(
        &mut self,
        result: std::result::Result<rmac_shortcuts::BackendStatus, rmac_shortcuts::Error>,
    ) {
        self.shortcut_status_loading = false;
        match result {
            Ok(status) => {
                self.shortcut_status = Some(status);
                self.shortcut_status_error = None;
            }
            Err(_) => {
                self.shortcut_status_error =
                    Some("The session shortcut broker has not reported its backend".into());
            }
        }
    }

    pub(super) fn refresh_shortcut_status(&mut self, cx: &mut Context<Self>) {
        if self.shortcut_status_loading {
            return;
        }
        self.shortcut_status_loading = true;
        self.shortcut_status_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(rmac_shortcuts::backend_status).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_shortcut_status_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn configure_global_shortcuts(&mut self, cx: &mut Context<Self>) {
        if self.shortcut_configuration_busy
            || !shortcut_configuration_available(self.shortcut_status.as_ref())
        {
            return;
        }
        self.shortcut_configuration_busy = true;
        self.shortcut_configuration_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result =
                blocking::unblock(rmac_shortcuts::request_shortcut_configuration).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.shortcut_configuration_busy = false;
                if result.is_err() {
                    this.shortcut_configuration_error = Some(
                        "Could not open global shortcut configuration. The live session broker or portal may be unavailable."
                            .into(),
                    );
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_spotlight(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let revert_view = view.clone();
        let choose_view = view.clone();
        let configure_shortcuts_view = view.clone();
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
        let enabled = !self.shell_settings_busy;
        let applications =
            spotlight_provider_policy(settings, rmac_launcher_providers::APPLICATIONS_PROVIDER);
        let settings_provider =
            spotlight_provider_policy(settings, rmac_launcher_providers::SETTINGS_PROVIDER);
        let files = spotlight_provider_policy(settings, rmac_launcher_providers::FILES_PROVIDER);
        let calculator =
            spotlight_provider_policy(settings, rmac_launcher_providers::CALCULATOR_PROVIDER);

        cards.push(section_header("Search results"));
        cards.push(card(vec![
            spotlight_provider_row(
                view.clone(),
                rmac_launcher_providers::APPLICATIONS_PROVIDER,
                "Applications",
                "Installed desktop applications",
                applications.enabled,
                enabled,
            ),
            spotlight_provider_row(
                view.clone(),
                rmac_launcher_providers::SETTINGS_PROVIDER,
                "System Settings",
                "Destinations and Linux-relevant setting keywords",
                settings_provider.enabled,
                enabled,
            ),
            spotlight_provider_row(
                view.clone(),
                rmac_launcher_providers::FILES_PROVIDER,
                "Files",
                if files.allow_private_content {
                    "On-demand filenames and recent documents"
                } else {
                    "Private-content permission is required"
                },
                files.enabled,
                enabled,
            ),
            spotlight_provider_row(
                view.clone(),
                rmac_launcher_providers::CALCULATOR_PROVIDER,
                "Calculator",
                "Local bounded arithmetic; no scripts or network",
                calculator.enabled,
                enabled,
            ),
        ]));

        cards.push(section_header("File privacy and scope"));
        let private_view = view.clone();
        let removable_view = view.clone();
        cards.push(card(vec![
            row_base()
                .child(text_block(
                    "Allow private file results".into(),
                    Some("Admit local filenames and recent-document paths to Search".into()),
                ))
                .child(
                    Toggle::new("spotlight-private-files")
                        .checked(files.allow_private_content)
                        .disabled(!enabled)
                        .on_click(move |value, _, cx| {
                            private_view.update(cx, |settings, cx| {
                                settings.apply_spotlight_change(
                                    SpotlightChange::ProviderPrivateContent {
                                        id: rmac_launcher_providers::FILES_PROVIDER.into(),
                                        allowed: *value,
                                    },
                                    cx,
                                )
                            });
                        }),
                )
                .into_any_element(),
            row_base()
                .child(text_block(
                    "Include removable mounts".into(),
                    Some("Allow on-demand file search to cross filesystem boundaries".into()),
                ))
                .child(
                    Toggle::new("spotlight-removable-mounts")
                        .checked(settings.spotlight.include_removable_mounts)
                        .disabled(!enabled)
                        .on_click(move |value, _, cx| {
                            removable_view.update(cx, |settings, cx| {
                                settings.apply_spotlight_change(
                                    SpotlightChange::IncludeRemovableMounts(*value),
                                    cx,
                                )
                            });
                        }),
                )
                .into_any_element(),
        ]));
        cards.push(note_card(
            "File search is local and on demand. rmac does not build a perpetual content index, and no built-in provider requests network access.",
        ));

        cards.push(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px_1()
                .pt_2()
                .pb_1()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .font_weight(rmac_ui::mac::SEMIBOLD)
                        .text_color(secondary())
                        .child("Excluded folders"),
                )
                .child(
                    Button::new("spotlight-add-exclusion", "Add Folder…")
                        .disabled(!enabled)
                        .on_click(move |_, _, cx| {
                            choose_view
                                .update(cx, |settings, cx| settings.choose_search_exclusion(cx));
                        }),
                ),
        );
        if settings.spotlight.excluded_paths.is_empty() {
            cards.push(note_card(
                "No folders are excluded. Add a folder to prune it before filename traversal and recent-document admission.",
            ));
        } else {
            let exclusion_rows = settings
                .spotlight
                .excluded_paths
                .iter()
                .enumerate()
                .map(|(index, path)| {
                    let remove_view = view.clone();
                    let remove_path = path.clone();
                    row_base()
                        .child(text_block(
                            PathBuf::from(path)
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.clone())
                                .into(),
                            Some(path.clone().into()),
                        ))
                        .child(
                            Button::new(
                                ElementId::from(SharedString::from(format!(
                                    "spotlight-remove-exclusion-{index}"
                                ))),
                                "Remove",
                            )
                            .disabled(!enabled)
                            .on_click(move |_, _, cx| {
                                remove_view.update(cx, |settings, cx| {
                                    settings.apply_spotlight_change(
                                        SpotlightChange::RemoveExclusion(remove_path.clone()),
                                        cx,
                                    )
                                });
                            }),
                        )
                        .into_any_element()
                })
                .collect();
            cards.push(card(exclusion_rows));
        }

        cards.push(section_header("Indexing"));
        cards.push(card(vec![
            value_row(
                "icons/search.svg",
                accent(),
                "Search mode".into(),
                "On demand".into(),
            ),
            value_row(
                "icons/hard-drive.svg",
                secondary(),
                "Filesystem scope".into(),
                if settings.spotlight.include_removable_mounts {
                    "Home and removable mounts".into()
                } else {
                    "Home filesystem only".into()
                },
            ),
            value_row(
                "icons/info.svg",
                secondary(),
                "Background content index".into(),
                "Not used".into(),
            ),
        ]));

        let launcher_shortcut = rmac_shortcuts::default_shortcuts()
            .into_iter()
            .find(|shortcut| shortcut.id.0 == "launcher")
            .expect("the stable launcher shortcut is registered");
        let shortcut_status: SharedString = match self.shortcut_status.as_ref() {
            Some(rmac_shortcuts::BackendStatus::Portal {
                version,
                can_configure,
            }) => format!(
                "Portal v{version}{}",
                if *can_configure && *version >= rmac_shortcuts::PORTAL_CONFIGURE_VERSION {
                    " · configurable"
                } else {
                    ""
                }
            )
            .into(),
            Some(rmac_shortcuts::BackendStatus::FallbackRequired { .. }) => {
                "niri fallback required".into()
            }
            None if self.shortcut_status_loading => "Loading…".into(),
            None => "Not reported".into(),
        };
        let shortcut_configuration_enabled =
            shortcut_configuration_available(self.shortcut_status.as_ref());
        let shortcut_configuration_detail = match self.shortcut_status.as_ref() {
            Some(rmac_shortcuts::BackendStatus::Portal {
                version,
                can_configure: true,
            }) if *version >= rmac_shortcuts::PORTAL_CONFIGURE_VERSION => {
                "Open the portal UI for every shortcut in the live rmac session"
            }
            Some(rmac_shortcuts::BackendStatus::Portal { .. }) => {
                "The active portal is older than GlobalShortcuts version 2"
            }
            Some(rmac_shortcuts::BackendStatus::FallbackRequired { .. }) => {
                "The generated niri fallback remains the shortcut authority"
            }
            None if self.shortcut_status_loading => "Waiting for the session broker",
            None => "The session broker has not reported its shortcut backend",
        };
        cards.push(section_header("Keyboard shortcut"));
        cards.push(card(vec![
            value_row(
                "icons/keyboard.svg",
                accent(),
                "Active backend".into(),
                shortcut_status,
            ),
            value_row(
                "icons/keyboard.svg",
                secondary(),
                "Portal preference".into(),
                launcher_shortcut.preferred_trigger.into(),
            ),
            value_row(
                "icons/keyboard.svg",
                secondary(),
                "niri fallback".into(),
                launcher_shortcut.niri_trigger.into(),
            ),
            row_base()
                .child(text_block(
                    "Global shortcuts".into(),
                    Some(shortcut_configuration_detail.into()),
                ))
                .child(
                    Button::new(
                        "spotlight-configure-shortcuts",
                        if self.shortcut_configuration_busy {
                            "Opening…"
                        } else {
                            "Configure…"
                        },
                    )
                    .disabled(self.shortcut_configuration_busy || !shortcut_configuration_enabled)
                    .on_click(move |_, _, cx| {
                        configure_shortcuts_view.update(cx, |settings, cx| {
                            settings.configure_global_shortcuts(cx);
                        });
                    }),
                )
                .into_any_element(),
        ]));
        if let Some(error) = self.shortcut_status_error.clone() {
            cards.push(note_card(error));
        }
        if let Some(error) = self.shortcut_configuration_error.clone() {
            cards.push(note_card(error));
        }
        cards.push(note_card(
            "The portal owns user consent and the actual trigger. Configure opens its UI through the broker's existing session; it never creates a second binding authority. The fallback is enabled only when the broker reports it is required, so one shortcut backend owns Logo/Mod+Space at a time.",
        ));
        cards.push(note_card(
            "These preferences are consumed by the launcher provider/runtime foundations. The centered GPUI overlay and full live session wiring remain D7/D8 release gates.",
        ));
        self.pane(cards)
    }
}
