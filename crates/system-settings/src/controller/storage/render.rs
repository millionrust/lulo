//! Storage settings presentation.

use super::*;

impl Settings {
    /// Direct per-volume capacity state from the mount service.
    pub(in crate::controller) fn storage_body(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let mut body = div().v_flex().child(card(vec![row_base()
            .child(tile("icons/hard-drive.svg", accent(), 22.0))
            .child(text_block(
                "Mounted volumes".into(),
                Some("System, removable, and network volumes".into()),
            ))
            .child(
                Button::new("refresh-storage", "Refresh")
                    .busy(self.storage_busy || self.storage_stream_refreshing)
                    .disabled(self.system_data_loading || self.storage_busy)
                    .on_click(move |_, _, cx| {
                        refresh_view.update(cx, |settings, cx| settings.refresh_storage(cx));
                    }),
            )
            .into_any_element()]));

        if self.storage.is_empty() {
            return body.child(
                EmptyState::new("No storage volumes available")
                    .message("Refresh after the mount service becomes available")
                    .error(self.storage_error.is_some() || self.storage_stream_error.is_some()),
            );
        }

        for (index, volume) in self.storage.iter().enumerate() {
            let identity = volume.mount.identity.clone();
            let action_busy = self.storage_action_busy.as_deref() == Some(identity.as_str());
            let action_disabled = self.storage_action_busy.is_some() || self.storage_busy;
            let action_view = view.clone();
            body = body.child(section_header(volume.mount.name.clone()));
            let Some(usage) = volume.usage else {
                body = body.child(card(vec![row_base()
                    .child(tile("icons/hard-drive.svg", hsl(0xff9500), 22.0))
                    .child(text_block(
                        volume.mount.name.clone().into(),
                        volume.usage_error.clone().map(Into::into),
                    ))
                    .child(
                        Button::new(("open-storage-volume", index), "Review in Files")
                            .busy(action_busy)
                            .disabled(action_disabled)
                            .on_click(move |_, _, cx| {
                                action_view.update(cx, |settings, cx| {
                                    settings.open_storage_volume(identity.clone(), cx);
                                });
                            }),
                    )
                    .into_any_element()]));
                continue;
            };
            let identity = volume.mount.identity.clone();
            let action_view = view.clone();
            let available_color = if usage.is_low_space() {
                hsl(0xff3b30)
            } else {
                hsl(0x34c759)
            };
            body = body
                .child(
                    div()
                        .v_flex()
                        .gap_2()
                        .mb_3()
                        .p_4()
                        .rounded(px(mac::radius_menu()))
                        .bg(card_bg())
                        .border_1()
                        .border_color(if usage.is_low_space() {
                            rmac_ui::mac::warning_border()
                        } else {
                            sep()
                        })
                        .child(
                            div()
                                .h_flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_size(rmac_ui::text_px(15.0))
                                        .font_weight(rmac_ui::mac::SEMIBOLD)
                                        .text_color(label())
                                        .child(volume.mount.name.clone()),
                                )
                                .child(
                                    div()
                                        .h_flex()
                                        .items_center()
                                        .gap_3()
                                        .child(
                                            div()
                                                .text_size(rmac_ui::text_px(13.0))
                                                .text_color(secondary())
                                                .child(format!(
                                                    "{} available of {}",
                                                    fmt_gb(usage.available),
                                                    fmt_gb(usage.total)
                                                )),
                                        )
                                        .child(
                                            Button::new(
                                                ("open-storage-volume", index),
                                                "Review in Files",
                                            )
                                            .busy(action_busy)
                                            .disabled(action_disabled)
                                            .on_click(
                                                move |_, _, cx| {
                                                    action_view.update(cx, |settings, cx| {
                                                        settings.open_storage_volume(
                                                            identity.clone(),
                                                            cx,
                                                        );
                                                    });
                                                },
                                            ),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .w_full()
                                .h(px(10.0))
                                .rounded(px(mac::radius_menu_item()))
                                .bg(rmac_ui::mac::control_fill())
                                .child(
                                    div()
                                        .h_full()
                                        .w(gpui::relative(usage.used_fraction()))
                                        .rounded(px(mac::radius_menu_item()))
                                        .bg(if usage.is_low_space() {
                                            hsl(0xff3b30)
                                        } else {
                                            accent()
                                        }),
                                ),
                        ),
                )
                .child(card(vec![
                    value_row(
                        "icons/database.svg",
                        secondary(),
                        "Capacity".into(),
                        fmt_gb(usage.total).into(),
                    ),
                    value_row(
                        "icons/database.svg",
                        hsl(0xff9500),
                        "Used".into(),
                        fmt_gb(usage.used).into(),
                    ),
                    value_row(
                        "icons/database.svg",
                        available_color,
                        "Available".into(),
                        fmt_gb(usage.available).into(),
                    ),
                ]));
            if usage.is_low_space() {
                body = body.child(note_card(
                    "Space is low on this volume. Review large personal files and application caches before removing anything; rmac does not guess which files are safe to delete.",
                ));
            }
        }
        body.child(note_card(
            "Review in Files opens only a currently revalidated mounted volume. Storage categories and destructive cleanup actions stay hidden until they can be measured and reversed safely.",
        ))
    }
}
