//! Wallpaper settings presentation.

use super::*;

impl Settings {
    pub(in crate::controller) fn render_wallpaper(&self, cx: &Context<Self>) -> Div {
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
            .rounded(px(rmac_ui::mac::radius_card()))
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
        let use_default_view = view.clone();
        let source_name = wallpaper_source_name(&selection);
        let current_builtin = match rmac_wallpaper::parse_source(selection.source.as_deref()) {
            Ok(rmac_wallpaper::Source::BuiltIn(id)) => Some(id),
            _ => None,
        };
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
        for id in rmac_wallpaper::BuiltInId::ALL {
            let using = current_builtin == Some(id);
            let row_view = view.clone();
            let change = if id == rmac_wallpaper::BuiltInId::Aurora {
                WallpaperChange::Source(None)
            } else {
                WallpaperChange::Source(Some(format!("builtin:{}", id.id())))
            };
            source_rows.push(
                row_base()
                    .child(text_block(
                        gpui::SharedString::from(format!("Original {}", id.metadata().title)),
                        Some("Procedural rmac artwork; no third-party file".into()),
                    ))
                    .child(
                        Button::new(
                            gpui::SharedString::from(format!("wallpaper-use-{}", id.id())),
                            "Use",
                        )
                        .disabled(!enabled || using)
                        .on_click(move |_, _, cx| {
                            let change = change.clone();
                            row_view.update(cx, |settings, cx| {
                                let target = settings.wallpaper_target.clone();
                                settings.apply_wallpaper_change(target, change, cx);
                            });
                        }),
                    )
                    .into_any_element(),
            );
        }
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
