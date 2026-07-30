//! Wallpaper preview, chooser, mutation, rollback, and pane authority.

use super::*;

impl Settings {
    pub(super) fn refresh_wallpaper_preview(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.shell_settings.as_ref() else {
            self.wallpaper_preview_loading = false;
            self.wallpaper_preview_error = Some("Wallpaper settings are unavailable".into());
            return;
        };
        let (selection, _) =
            wallpaper_selection(&snapshot.settings.wallpaper, &self.wallpaper_target);
        let watched_paths = rmac_wallpaper::parse_source(selection.source.as_deref())
            .ok()
            .and_then(|source| rmac_wallpaper::file_path(&source).map(PathBuf::from))
            .into_iter()
            .collect::<Vec<_>>();
        self.wallpaper_preview_generation = self.wallpaper_preview_generation.wrapping_add(1);
        let generation = self.wallpaper_preview_generation;
        self.wallpaper_preview_loading = true;
        self.wallpaper_preview_error = None;
        self.wallpaper_preview_watch_error = None;
        self._wallpaper_preview_watcher = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (events_tx, events_rx) = async_channel::bounded(1);
            let (result, watcher) = blocking::unblock(move || {
                let watcher = rmac_wallpaper_image::watch_files(&watched_paths, events_tx);
                (render_wallpaper_preview(&selection), watcher)
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.wallpaper_preview_generation != generation {
                    return;
                }
                this.wallpaper_preview_loading = false;
                match result {
                    Ok(preview) => {
                        this.wallpaper_preview = Some(preview);
                        this.wallpaper_preview_error = None;
                    }
                    Err(error) => {
                        this.wallpaper_preview_error =
                            Some(format!("Could not preview this wallpaper: {error}").into());
                    }
                }
                let watching = match watcher {
                    Ok(watcher) => {
                        let watching = watcher.is_some();
                        this._wallpaper_preview_watcher = watcher;
                        this.wallpaper_preview_watch_error = None;
                        watching
                    }
                    Err(_) => {
                        this.wallpaper_preview_watch_error = Some(
                            "Live updates for the selected wallpaper file are unavailable".into(),
                        );
                        false
                    }
                };
                if watching {
                    cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                        let event = events_rx.recv().await;
                        let _ = this.update(cx, |this: &mut Settings, cx| {
                            if this.wallpaper_preview_generation != generation {
                                return;
                            }
                            match event {
                                Ok(rmac_wallpaper_image::FileWatchEvent::Changed) => {
                                    this.refresh_wallpaper_preview(cx);
                                }
                                Ok(rmac_wallpaper_image::FileWatchEvent::Failed { .. })
                                | Err(_) => {
                                    this._wallpaper_preview_watcher = None;
                                    this.wallpaper_preview_watch_error = Some(
                                        "Live updates for the selected wallpaper file stopped"
                                            .into(),
                                    );
                                    cx.notify();
                                }
                            }
                        });
                    })
                    .detach();
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn select_wallpaper_target(
        &mut self,
        target: WallpaperTarget,
        cx: &mut Context<Self>,
    ) {
        if self.shell_settings_busy || self.wallpaper_target == target {
            return;
        }
        self.wallpaper_target = target;
        self.wallpaper_error = None;
        self.refresh_wallpaper_preview(cx);
    }

    pub(super) fn apply_wallpaper_change(
        &mut self,
        target: WallpaperTarget,
        change: WallpaperChange,
        cx: &mut Context<Self>,
    ) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            return;
        };
        let previous = snapshot.settings.wallpaper.clone();
        let mut next = previous.clone();
        change.clone().apply(&target, &mut next);
        if next == previous {
            return;
        }

        self.shell_settings_busy = true;
        self.wallpaper_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::Wallpaper { target, change })
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wallpaper_mutation(result, Some(previous));
                this.refresh_wallpaper_preview(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn finish_wallpaper_mutation(
        &mut self,
        result: std::result::Result<rmac_shell_settings::Snapshot, rmac_shell_settings::Error>,
        previous: Option<rmac_shell_settings::WallpaperSettings>,
    ) {
        self.shell_settings_loading = false;
        self.shell_settings_busy = false;
        match result {
            Ok(snapshot) => {
                if self
                    .shell_settings
                    .as_ref()
                    .is_some_and(|current| current.settings.dock != snapshot.settings.dock)
                {
                    self.shell_settings_revert = None;
                }
                if self.shell_settings.as_ref().is_some_and(|current| {
                    SpotlightAuthority::from_settings(&current.settings)
                        != SpotlightAuthority::from_settings(&snapshot.settings)
                }) {
                    self.spotlight_revert = None;
                }
                self.shell_settings = Some(snapshot);
                self.wallpaper_revert = previous;
                self.wallpaper_error = None;
                self.shell_settings_error = None;
                self.shell_settings_stream_error = None;
            }
            Err(error) => {
                self.wallpaper_error = Some(format!("Could not update Wallpaper: {error}").into());
            }
        }
    }

    pub(super) fn choose_wallpaper_file(&mut self, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            return;
        };
        let target = self.wallpaper_target.clone();
        let fit = wallpaper_selection(&snapshot.settings.wallpaper, &target)
            .0
            .fit;
        self.shell_settings_busy = true;
        self.wallpaper_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let choice = rmac_portal::choose_wallpaper_file().await;
            let validated = match choice {
                Ok(Some(path)) => Some(
                    cx.background_executor()
                        .spawn(async move {
                            blocking::unblock(move || validate_wallpaper_choice(path, fit)).await
                        })
                        .await,
                ),
                Ok(None) => None,
                Err(error) => Some(Err(error.to_string())),
            };
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.shell_settings_busy = false;
                match validated {
                    Some(Ok(source)) => this.apply_wallpaper_change(
                        target,
                        WallpaperChange::Source(Some(source)),
                        cx,
                    ),
                    Some(Err(error)) => {
                        this.wallpaper_error = Some(error.into());
                    }
                    None => this.refresh_shell_settings(true, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn revert_wallpaper_change(&mut self, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(previous) = self.wallpaper_revert.clone() else {
            return;
        };
        self.shell_settings_busy = true;
        self.wallpaper_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::RestoreWallpaper(previous))
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wallpaper_mutation(result, None);
                this.refresh_wallpaper_preview(cx);
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn render_wallpaper(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let revert_view = view.clone();
        let choose_view = view.clone();
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
                    .child("Desktop wallpaper"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("wallpaper-revert", "Revert")
                            .disabled(
                                self.shell_settings_loading
                                    || self.shell_settings_busy
                                    || self.wallpaper_revert.is_none(),
                            )
                            .on_click(move |_, _, cx| {
                                revert_view.update(cx, |settings, cx| {
                                    settings.revert_wallpaper_change(cx)
                                });
                            }),
                    )
                    .child(
                        Button::new("wallpaper-choose", "Choose Image…")
                            .disabled(self.shell_settings_loading || self.shell_settings_busy)
                            .on_click(move |_, _, cx| {
                                choose_view
                                    .update(cx, |settings, cx| settings.choose_wallpaper_file(cx));
                            }),
                    )
                    .child(
                        Button::new(
                            "wallpaper-refresh",
                            if self.shell_settings_busy {
                                "Applying…"
                            } else if self.shell_settings_loading {
                                "Loading…"
                            } else {
                                "Refresh"
                            },
                        )
                        .disabled(self.shell_settings_loading || self.shell_settings_busy)
                        .on_click(move |_, _, cx| {
                            refresh_view.update(cx, |settings, cx| {
                                settings.refresh_shell_settings(true, cx)
                            });
                        }),
                    ),
            )];

        if self.shell_settings_loading && self.shell_settings.is_none() {
            cards.push(note_card("Loading the authoritative wallpaper settings…"));
            return self.pane(cards);
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            cards.push(note_card(
                "The versioned rmac shell-settings authority is unavailable. Wallpaper choices remain unchanged.",
            ));
            return self.pane(cards);
        };
        let wallpaper = &snapshot.settings.wallpaper;
        let (selection, owns_selection) = wallpaper_selection(wallpaper, &self.wallpaper_target);
        let enabled = !self.shell_settings_busy;

        cards.push(section_header("Apply to"));
        let mut targets = div().flex().flex_wrap().gap_2().mb_3();
        let default_view = view.clone();
        targets = targets.child(
            Button::new("wallpaper-target-default", "All displays (default)")
                .selected(self.wallpaper_target == WallpaperTarget::Default)
                .disabled(!enabled)
                .on_click(move |_, _, cx| {
                    default_view.update(cx, |settings, cx| {
                        settings.select_wallpaper_target(WallpaperTarget::Default, cx)
                    });
                }),
        );
        let mut output_ids = std::collections::BTreeSet::new();
        output_ids.extend(wallpaper.per_output.keys().cloned());
        if let WallpaperTarget::Output(output) = &self.wallpaper_target {
            output_ids.insert(output.clone());
        }
        output_ids.extend(
            self.dock_compositor
                .outputs
                .values()
                .filter(|output| output.enabled())
                .map(|output| output.id.0.clone()),
        );
        for output_id in output_ids {
            let target = WallpaperTarget::Output(output_id.clone());
            let selected = self.wallpaper_target == target;
            let live = self
                .dock_compositor
                .outputs
                .values()
                .any(|output| output.enabled() && output.id.0 == output_id);
            let label = self
                .dock_compositor
                .outputs
                .get(&rmac_compositor::OutputId(output_id.clone()))
                .map(|output| {
                    format!("{} {}", output.make, output.model)
                        .trim()
                        .to_owned()
                })
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| output_id.clone());
            let target_view = view.clone();
            targets = targets.child(
                Button::new(
                    ElementId::from(SharedString::from(format!("wallpaper-target-{output_id}"))),
                    if live {
                        label
                    } else {
                        format!("{label} · offline")
                    },
                )
                .selected(selected)
                .disabled(!enabled)
                .on_click(move |_, _, cx| {
                    target_view.update(cx, |settings, cx| {
                        settings.select_wallpaper_target(target.clone(), cx)
                    });
                }),
            );
        }
        cards.push(targets);

        cards.push(section_header("Preview"));
        let preview = div()
            .w(px(480.0))
            .h(px(270.0))
            .mx_auto()
            .mb_3()
            .rounded(px(12.0))
            .overflow_hidden()
            .bg(hsl(0x1e1e20))
            .border_1()
            .border_color(sep())
            .when_some(self.wallpaper_preview.clone(), |element, preview| {
                element.child(img(preview).w_full().h_full().object_fit(ObjectFit::Fill))
            })
            .when(self.wallpaper_preview.is_none(), |element| {
                element.flex().items_center().justify_center().child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(white())
                        .child(if self.wallpaper_preview_loading {
                            "Preparing preview…"
                        } else {
                            "Preview unavailable"
                        }),
                )
            });
        cards.push(preview);
        if self.wallpaper_preview_loading && self.wallpaper_preview.is_some() {
            cards.push(note_card(
                "Refreshing the preview from the selected source…",
            ));
        }
        if let Some(error) = self.wallpaper_preview_error.clone() {
            cards.push(note_card(error));
        }
        if let Some(error) = self.wallpaper_preview_watch_error.clone() {
            cards.push(note_card(error));
        }

        cards.push(section_header("Image"));
        let aurora_view = view.clone();
        let use_default_view = view.clone();
        let source_name = wallpaper_source_name(&selection);
        let using_aurora = matches!(
            rmac_wallpaper::parse_source(selection.source.as_deref()),
            Ok(rmac_wallpaper::Source::BuiltIn(_))
        );
        let mut source_rows = vec![row_base()
            .child(text_block(
                "Current image".into(),
                Some(match &self.wallpaper_target {
                    WallpaperTarget::Default => "Default for every display".into(),
                    WallpaperTarget::Output(_) if owns_selection => {
                        "Custom choice for this output".into()
                    }
                    WallpaperTarget::Output(_) => "Inherited from the default".into(),
                }),
            ))
            .child(
                div()
                    .max_w(px(190.0))
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(secondary())
                    .child(source_name),
            )
            .into_any_element()];
        source_rows.push(
            row_base()
                .child(text_block(
                    "Original Aurora".into(),
                    Some("Procedural rmac artwork; no third-party file".into()),
                ))
                .child(
                    Button::new("wallpaper-use-aurora", "Use")
                        .disabled(!enabled || using_aurora)
                        .on_click(move |_, _, cx| {
                            aurora_view.update(cx, |settings, cx| {
                                let target = settings.wallpaper_target.clone();
                                settings.apply_wallpaper_change(
                                    target,
                                    WallpaperChange::Source(None),
                                    cx,
                                )
                            });
                        }),
                )
                .into_any_element(),
        );
        if matches!(self.wallpaper_target, WallpaperTarget::Output(_)) {
            source_rows.push(
                row_base()
                    .child(text_block(
                        "Use default wallpaper".into(),
                        Some("Remove this output's saved override".into()),
                    ))
                    .child(
                        Button::new("wallpaper-use-default", "Use Default")
                            .disabled(!enabled || !owns_selection)
                            .on_click(move |_, _, cx| {
                                use_default_view.update(cx, |settings, cx| {
                                    let target = settings.wallpaper_target.clone();
                                    settings.apply_wallpaper_change(
                                        target,
                                        WallpaperChange::UseDefault,
                                        cx,
                                    )
                                });
                            }),
                    )
                    .into_any_element(),
            );
        }
        cards.push(card(source_rows));

        cards.push(section_header("Fit"));
        cards.push(card(vec![wallpaper_fit_row(
            view.clone(),
            selection.fit,
            enabled,
        )]));

        let connection = match self.dock_compositor.connection {
            rmac_compositor::ConnectionState::Connected => "Connected",
            rmac_compositor::ConnectionState::Connecting => "Connecting",
            rmac_compositor::ConnectionState::Reconnecting => "Reconnecting",
            rmac_compositor::ConnectionState::Disconnected => "Unavailable",
        };
        cards.push(section_header("Authority"));
        cards.push(card(vec![
            value_row(
                "icons/settings.svg",
                accent(),
                "Saved preferences".into(),
                "C4 shell settings".into(),
            ),
            value_row(
                "icons/monitor.svg",
                secondary(),
                "niri output stream".into(),
                connection.into(),
            ),
            value_row(
                "icons/image.svg",
                secondary(),
                "Accepted image types".into(),
                "PNG, JPEG, WebP".into(),
            ),
        ]));
        if let WallpaperTarget::Output(output) = &self.wallpaper_target {
            let live = self
                .dock_compositor
                .outputs
                .values()
                .any(|candidate| candidate.enabled() && candidate.id.0 == *output);
            if !live {
                cards.push(note_card(
                    "This output is currently unplugged or disabled. Its override remains authoritative and will return when the same stable niri output ID reappears.",
                ));
            }
        }
        if self.dock_compositor.connection != rmac_compositor::ConnectionState::Connected {
            cards.push(note_card(
                "niri is not connected in this process. Saved per-output choices remain editable, but live output availability cannot be confirmed.",
            ));
        }
        cards.push(note_card(
            "The preview uses the same bounded PNG/JPEG/WebP decoder and exact Fill, Fit, Stretch, Center, or Tile geometry as the wallpaper runtime. The Wayland background surface itself remains a separate D9 release gate.",
        ));
        self.pane(cards)
    }
}
