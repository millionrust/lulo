//! Storage settings presentation (design-lab/settings.html › Storage).

use super::*;

impl Settings {
    /// macOS 26 Storage: the home volume's bar split by measured category
    /// with its legend, one row per category, then every other volume.
    pub(in crate::controller) fn storage_body(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = push_button("refresh-storage", "Refresh")
            .busy(
                self.storage_busy || self.storage_stream_refreshing || self.storage_categories_busy,
            )
            .disabled(self.system_data_loading || self.storage_busy)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_storage(cx));
            })
            .into_any_element();

        let mut body = div().v_flex();
        if self.storage.is_empty() {
            return body
                .child(
                    EmptyState::new("No storage volumes available")
                        .message("Refresh after the mount service becomes available")
                        .error(self.storage_error.is_some() || self.storage_stream_error.is_some()),
                )
                .child(footer_buttons(vec![refresh]));
        }

        let home_identity = self
            .home_volume()
            .map(|volume| volume.mount.identity.clone());
        let categories = self.storage_categories.as_ref().filter(|categories| {
            self.home_volume()
                .is_some_and(|volume| volume.mount.path == categories.volume_path)
        });

        // The home volume first, as the Mac puts the startup disk first.
        let mut volumes: Vec<&rmac_mounts::Volume> = self.storage.iter().collect();
        volumes.sort_by_key(|volume| Some(&volume.mount.identity) != home_identity.as_ref());

        for (index, volume) in volumes.into_iter().enumerate() {
            let is_home = Some(&volume.mount.identity) == home_identity.as_ref();
            let identity = volume.mount.identity.clone();
            let action_busy = self.storage_action_busy.as_deref() == Some(identity.as_str());
            let action_disabled = self.storage_action_busy.is_some() || self.storage_busy;
            let Some(usage) = volume.usage else {
                let open_view = view.clone();
                body = body.child(
                    group().child(value_button_row(
                        volume.mount.name.clone(),
                        volume.usage_error.as_ref().map(|error| {
                            rmac_ui::user_error_message(
                                rmac_ui::ErrorSurface::Settings,
                                error.as_ref(),
                                false,
                            )
                        }),
                        None,
                        Some(
                            push_button(("open-storage-volume", index), "Show in Files")
                                .busy(action_busy)
                                .disabled(action_disabled)
                                .on_click(move |_, _, cx| {
                                    open_view.update(cx, |settings, cx| {
                                        settings.open_storage_volume(identity.clone(), cx);
                                    });
                                })
                                .into_any_element(),
                        ),
                    )),
                );
                continue;
            };

            let total = usage.total.max(1) as f32;
            let mut segments = Vec::new();
            let mut legend = Vec::new();
            if let (true, Some(categories)) = (is_home, categories) {
                for (position, category) in categories.categories.iter().enumerate() {
                    let color = hsl(style::STORAGE_COLORS[position % style::STORAGE_COLORS.len()]);
                    segments.push((color, category.bytes as f32 / total));
                    legend.push((color, SharedString::from(category.name)));
                }
                segments.push((
                    style::storage_system_data(),
                    categories.system_data(usage.used) as f32 / total,
                ));
                legend.push((style::storage_system_data(), "System Data".into()));
            } else {
                segments.push((style::storage_system_data(), usage.used_fraction()));
            }

            let mut card_body = group()
                .pt(px(12.0))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .px(px(style::ROW_PADDING))
                        .pb(px(12.0))
                        .text_size(rmac_ui::text_px(13.0))
                        .line_height(px(16.0))
                        .child(div().text_color(label()).child(volume.mount.name.clone()))
                        .child(div().text_color(secondary()).child(format!(
                            "{} of {} used",
                            fmt_gb(usage.used),
                            fmt_gb(usage.total)
                        ))),
                )
                .child(storage_bar(
                    &segments,
                    usage.available as f32 / total,
                    fmt_gb(usage.available),
                ));
            card_body = if legend.is_empty() {
                card_body.child(div().h(px(style::ROW_PADDING)))
            } else {
                card_body.child(storage_legend(legend))
            };
            body = body.child(card_body);

            if is_home {
                match categories {
                    Some(categories) => {
                        let mut rows = categories
                            .categories
                            .iter()
                            .enumerate()
                            .map(|(position, category)| {
                                let color =
                                    hsl(style::STORAGE_COLORS
                                        [position % style::STORAGE_COLORS.len()]);
                                let row = icon_row(
                                    tile(
                                        storage_category_icon(category.name),
                                        color,
                                        style::ROW_ICON,
                                    )
                                    .into_any_element(),
                                    category.name,
                                )
                                .child(trailing_value(fmt_gb(category.bytes)));
                                match category.folder.clone() {
                                    Some(folder) => {
                                        let reveal_view = view.clone();
                                        row.child(info_button(
                                            SharedString::from(format!(
                                                "storage-category-{}",
                                                category.name
                                            )),
                                            "Show in Files",
                                            move |_, cx| {
                                                let folder = folder.clone();
                                                reveal_view.update(cx, |settings, cx| {
                                                    settings.reveal_storage_category(folder, cx)
                                                });
                                            },
                                        ))
                                        .into_any_element()
                                    }
                                    None => row.into_any_element(),
                                }
                            })
                            .collect::<Vec<_>>();
                        rows.push(
                            icon_row(
                                tile("icons/settings.svg", secondary(), style::ROW_ICON)
                                    .into_any_element(),
                                "System Data",
                            )
                            .child(trailing_value(fmt_gb(categories.system_data(usage.used))))
                            .into_any_element(),
                        );
                        body = body.child(card(rows));
                        if categories.truncated {
                            body = body.child(footnote(
                                "Some folders hold more files than Storage measures; their sizes are at least what is shown.",
                            ));
                        }
                    }
                    None if self.storage_categories_busy => {
                        body = body.child(group().child(group_placeholder("Calculating…")));
                    }
                    None => {}
                }
            }
            if usage.is_low_space() {
                body = body.child(footnote(format!(
                    "{} is almost full. Review large files before removing anything.",
                    volume.mount.name
                )));
            }
        }
        body.child(footer_buttons(vec![refresh]))
    }
}

/// The glyph for a measured Storage category.
fn storage_category_icon(name: &str) -> &'static str {
    match name {
        "Applications" => "icons/app-window.svg",
        "Documents" => "icons/folder-symlink.svg",
        "Music" => "icons/volume-2.svg",
        "Photos" => "icons/image.svg",
        "Movies" => "icons/monitor.svg",
        "Bin" => "icons/history.svg",
        _ => "icons/database.svg",
    }
}
