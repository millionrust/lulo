//! Wallpaper settings presentation, laid out like macOS 26
//! (design-lab/settings.html): the current wallpaper's preview beside a
//! group with its name, fit and target display, the pane's buttons under
//! it, then the built-in wallpapers as a thumbnail grid.

use super::*;

/// Measured: preview 160 × 100 at the content's left edge, the group 10
/// after it; thumbnails 108 wide on a 118 pt pitch with the name under a
/// 68 pt picture.
const PREVIEW_WIDTH: f32 = 160.0;
const PREVIEW_HEIGHT: f32 = 100.0;
const THUMB_WIDTH: f32 = 108.0;
const THUMB_HEIGHT: f32 = 68.0;
const THUMB_GAP: f32 = 9.0;

impl Settings {
    pub(in crate::controller) fn render_wallpaper(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let revert_view = view.clone();
        let choose_view = view.clone();
        let reload_view = view.clone();
        let mut cards = Vec::new();

        if self.shell_settings_loading && self.shell_settings.is_none() {
            cards.push(note_card("Loading the authoritative wallpaper settings…"));
            return self.pane(cards);
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            cards.push(note_card(
                "The versioned rmac shell-settings authority is unavailable. Wallpaper choices remain unchanged.",
            ));
            cards.push(footer_buttons(vec![push_button(
                "wallpaper-refresh",
                "Refresh",
            )
            .disabled(self.shell_settings_loading || self.shell_settings_busy)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_shell_settings(true, cx));
            })
            .into_any_element()]));
            return self.pane(cards);
        };
        let wallpaper = &snapshot.settings.wallpaper;
        let (selection, owns_selection) = wallpaper_selection(wallpaper, &self.wallpaper_target);
        let enabled = !self.shell_settings_busy;

        // Target display: every saved override plus every enabled output.
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
        let default_view = view.clone();
        let mut targets = vec![choice(
            "All Displays",
            self.wallpaper_target == WallpaperTarget::Default,
            move |_, cx| {
                default_view.update(cx, |settings, cx| {
                    settings.select_wallpaper_target(WallpaperTarget::Default, cx)
                });
            },
        )];
        for output_id in output_ids {
            let target = WallpaperTarget::Output(output_id.clone());
            let selected = self.wallpaper_target == target;
            let live = self
                .dock_compositor
                .outputs
                .values()
                .any(|output| output.enabled() && output.id.0 == output_id);
            let name = self
                .dock_compositor
                .outputs
                .get(&rmac_compositor::OutputId(output_id.clone()))
                .map(|output| {
                    format!("{} {}", output.make, output.model)
                        .trim()
                        .to_owned()
                })
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| output_id.clone());
            let target_view = view.clone();
            targets.push(choice(
                if live {
                    name
                } else {
                    format!("{name} (Offline)")
                },
                selected,
                move |_, cx| {
                    target_view.update(cx, |settings, cx| {
                        settings.select_wallpaper_target(target.clone(), cx)
                    });
                },
            ));
        }
        let current_target = popup_value(&targets, "All Displays");

        let preview = div()
            .w(px(PREVIEW_WIDTH))
            .h(px(PREVIEW_HEIGHT))
            .flex_none()
            .rounded(px(8.0))
            .overflow_hidden()
            .bg(style::well_fill())
            .when_some(self.wallpaper_preview.clone(), |element, preview| {
                element.child(img(preview).w_full().h_full().object_fit(ObjectFit::Cover))
            })
            .when(self.wallpaper_preview.is_none(), |element| {
                element.flex().items_center().justify_center().child(
                    div()
                        .text_size(rmac_ui::text_px(11.0))
                        .text_color(secondary())
                        .child(if self.wallpaper_preview_loading {
                            "Preparing preview…"
                        } else {
                            "Preview unavailable"
                        }),
                )
            });
        let source_name = wallpaper_source_name(&selection);
        let summary = card(vec![
            wallpaper_fit_row(view.clone(), source_name.clone(), selection.fit, enabled),
            popup_row(
                "wallpaper-target",
                "Show on",
                Some(match &self.wallpaper_target {
                    WallpaperTarget::Default => "Default for every display".into(),
                    WallpaperTarget::Output(_) if owns_selection => {
                        "Custom choice for this display".into()
                    }
                    WallpaperTarget::Output(_) => "Inherited from the default".into(),
                }),
                current_target,
                targets,
                enabled,
            ),
        ]);
        cards.push(
            div()
                .flex()
                .items_start()
                .gap(px(style::GROUP_GAP))
                .pt(px(13.0))
                .child(preview)
                .child(div().flex_1().min_w_0().child(summary)),
        );

        let use_default_view = view.clone();
        let mut buttons = vec![
            push_button("wallpaper-choose", "Choose Image…")
                .disabled(self.shell_settings_loading || self.shell_settings_busy)
                .on_click(move |_, _, cx| {
                    choose_view.update(cx, |settings, cx| settings.choose_wallpaper_file(cx));
                })
                .into_any_element(),
            push_button("wallpaper-revert", "Revert")
                .disabled(
                    self.shell_settings_loading
                        || self.shell_settings_busy
                        || self.wallpaper_revert.is_none(),
                )
                .on_click(move |_, _, cx| {
                    revert_view.update(cx, |settings, cx| settings.revert_wallpaper_change(cx));
                })
                .into_any_element(),
            push_button(
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
                reload_view.update(cx, |settings, cx| settings.refresh_shell_settings(true, cx));
            })
            .into_any_element(),
        ];
        if matches!(self.wallpaper_target, WallpaperTarget::Output(_)) {
            buttons.insert(
                0,
                push_button("wallpaper-use-default", "Use Default")
                    .disabled(!enabled || !owns_selection)
                    .on_click(move |_, _, cx| {
                        use_default_view.update(cx, |settings, cx| {
                            let target = settings.wallpaper_target.clone();
                            settings.apply_wallpaper_change(target, WallpaperChange::UseDefault, cx)
                        });
                    })
                    .into_any_element(),
            );
        }
        cards.push(footer_buttons(buttons));

        if self.wallpaper_preview_loading && self.wallpaper_preview.is_some() {
            cards.push(note_card(
                "Refreshing the preview from the selected source…",
            ));
        }
        for error in [
            self.wallpaper_preview_error.clone(),
            self.wallpaper_preview_watch_error.clone(),
        ]
        .into_iter()
        .flatten()
        {
            cards.push(note_card(rmac_ui::user_error_message(
                rmac_ui::ErrorSurface::Settings,
                error.as_ref(),
                false,
            )));
        }

        // The built-in wallpapers: each thumbnail is drawn from that
        // wallpaper's own procedural palette for the current appearance.
        let current_builtin = match rmac_wallpaper::parse_source(selection.source.as_deref()) {
            Ok(rmac_wallpaper::Source::BuiltIn(id)) => Some(id),
            _ => None,
        };
        let mut grid = div().flex().flex_wrap().gap(px(THUMB_GAP));
        for id in rmac_wallpaper::BuiltInId::ALL {
            let metadata = id.metadata();
            let palette = metadata.palette_for(style::dark());
            let using = current_builtin == Some(id);
            let row_view = view.clone();
            let change = if id == rmac_wallpaper::BuiltInId::Aurora {
                WallpaperChange::Source(None)
            } else {
                WallpaperChange::Source(Some(format!("builtin:{}", id.id())))
            };
            grid = grid.child(
                div()
                    .id(SharedString::from(format!("wallpaper-use-{}", id.id())))
                    .w(px(THUMB_WIDTH))
                    .v_flex()
                    .items_center()
                    .gap(px(5.0))
                    .when(enabled && !using, |thumb| thumb.cursor_pointer())
                    .child(
                        div()
                            .w_full()
                            .h(px(THUMB_HEIGHT))
                            .rounded(px(8.0))
                            .bg(gpui::linear_gradient(
                                135.0,
                                gpui::linear_color_stop(hsl(palette[0]), 0.0),
                                gpui::linear_color_stop(hsl(palette[3]), 1.0),
                            ))
                            .when(using, |picture| picture.border_2().border_color(accent())),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .line_height(px(14.0))
                            .text_color(label())
                            .child(metadata.title),
                    )
                    .when(enabled && !using, |thumb| {
                        thumb.on_click(move |_, _, cx| {
                            let change = change.clone();
                            row_view.update(cx, |settings, cx| {
                                let target = settings.wallpaper_target.clone();
                                settings.apply_wallpaper_change(target, change, cx);
                            });
                        })
                    }),
            );
        }
        cards.push(section_header("Wallpapers"));
        cards.push(grid);

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
        self.pane(cards)
    }
}
