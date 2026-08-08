//! Open-at-login inventory and review projection.

use super::*;

impl Settings {
    pub(super) fn append_open_at_login(
        &self,
        view: Entity<Self>,
        snapshot: &rmac_login_items::Snapshot,
        cards: &mut Vec<Div>,
    ) {
        let choose_view = view.clone();
        cards.push(section_header("Open at login"));
        cards.push(card(vec![row_base()
            .child(tile("icons/app-window.svg", accent(), 22.0))
            .child(text_block(
                "Add application entry".into(),
                Some("Choose a local .desktop file to review".into()),
            ))
            .child(
                Button::new("choose-login-item", "Add…")
                    .busy(self.login_item_busy.as_deref() == Some("choose"))
                    .disabled(self.login_item_busy.is_some())
                    .on_click(move |_, _, cx| {
                        choose_view.update(cx, |settings, cx| settings.choose_login_item(cx));
                    }),
            )
            .into_any_element()]));
        if let Some(preview) = &self.login_item_add {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(note_card(format!(
                "Review “{}” ({}). This command will be allowed to run at sign-in. {}",
                preview.name,
                preview.id,
                if preview.replacing {
                    "A user entry with this filename exists and will be replaced only after confirmation."
                } else {
                    "The validated entry will be copied into your user autostart directory."
                }
            )));
            cards.push(card(vec![row_base()
                .child(tile("icons/info.svg", rmac_ui::mac::warning_text(), 22.0))
                .child(text_block(
                    "Command at sign-in".into(),
                    Some(preview.command.clone().into()),
                ))
                .into_any_element()]));
            cards.push(card(vec![row_base()
                .child(tile("icons/info.svg", secondary(), 22.0))
                .child(text_block(
                    if preview.replacing {
                        "Replace existing login item"
                    } else {
                        "Add login item"
                    }
                    .into(),
                    Some("The installed copy will start enabled".into()),
                ))
                .child(
                    Button::new("cancel-add-login-item", "Cancel")
                        .disabled(self.login_item_busy.is_some())
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| {
                                settings.login_item_add = None;
                                cx.notify();
                            });
                        }),
                )
                .child(
                    Button::new(
                        "confirm-add-login-item",
                        if preview.replacing { "Replace" } else { "Add" },
                    )
                    .primary()
                    .busy(self.login_item_busy.as_deref() == Some("add"))
                    .disabled(self.login_item_busy.is_some())
                    .on_click(move |_, _, cx| {
                        confirm_view.update(cx, |settings, cx| settings.confirm_add_login_item(cx));
                    }),
                )
                .into_any_element()]));
        }
        if snapshot.items.is_empty() {
            cards.push(note_card("No effective XDG autostart entries were found."));
        } else {
            let rows = snapshot
                .items
                .iter()
                .map(|item| {
                    let id = item.id.clone();
                    let reveal_id = item.id.clone();
                    let toggle_view = view.clone();
                    let reveal_view = view.clone();
                    let remove_view = view.clone();
                    let remove_id = item.id.clone();
                    let busy = self.login_item_busy.as_deref() == Some(item.id.as_str());
                    let remove_key = format!("prepare-remove:{}", item.id);
                    let preparing_remove =
                        self.login_item_busy.as_deref() == Some(remove_key.as_str());
                    let reveal_key = format!("reveal:{}", item.id);
                    let revealing = self.login_item_busy.as_deref() == Some(reveal_key.as_str());
                    let subtitle = item.session_detail.clone().unwrap_or_else(|| {
                        if item.user_owned {
                            "User autostart entry".into()
                        } else {
                            "System autostart entry".into()
                        }
                    });
                    row_base()
                        .child(tile("icons/app-window.svg", accent(), 22.0))
                        .child(text_block(item.name.clone().into(), Some(subtitle.into())))
                        .when(item.user_owned && !item.managed_override, |row| {
                            row.child(
                                Button::new(
                                    ElementId::from(SharedString::from(format!(
                                        "remove-login-item-{}",
                                        item.id
                                    ))),
                                    "Remove…",
                                )
                                .busy(preparing_remove)
                                .disabled(self.login_item_busy.is_some())
                                .on_click(move |_, _, cx| {
                                    remove_view.update(cx, |settings, cx| {
                                        settings.request_remove_login_item(remove_id.clone(), cx);
                                    });
                                }),
                            )
                        })
                        .child(
                            Button::new(
                                ElementId::from(SharedString::from(format!(
                                    "reveal-login-item-{}",
                                    item.id
                                ))),
                                "Show in Files",
                            )
                            .busy(revealing)
                            .disabled(self.login_item_busy.is_some())
                            .on_click(move |_, _, cx| {
                                reveal_view.update(cx, |settings, cx| {
                                    settings.reveal_login_item(reveal_id.clone(), false, cx);
                                });
                            }),
                        )
                        .child(
                            Toggle::new(ElementId::from(SharedString::from(format!(
                                "login-item-{}",
                                item.id
                            ))))
                            .checked(item.enabled)
                            .disabled(self.login_item_busy.is_some() || !item.can_toggle)
                            .on_click(move |enabled, _, cx| {
                                toggle_view.update(cx, |settings, cx| {
                                    settings.set_login_item_enabled(id.clone(), *enabled, cx);
                                });
                            }),
                        )
                        .when(busy, |row| {
                            row.child(
                                div()
                                    .text_size(rmac_ui::text_px(11.0))
                                    .text_color(secondary())
                                    .child("Saving…"),
                            )
                        })
                        .into_any_element()
                })
                .collect();
            cards.push(card(rows));
        }
        if let Some(preview) = &self.login_item_remove {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(note_card(format!(
                "Remove “{}”? Its user-owned desktop entry will be moved to Trash. If a system entry with the same filename exists, it will remain visible but disabled.",
                preview.name
            )));
            cards.push(card(vec![row_base()
                .child(tile("icons/info.svg", rmac_ui::mac::warning_text(), 22.0))
                .child(text_block(
                    "Confirm removal".into(),
                    Some("This does not delete the application itself".into()),
                ))
                .child(
                    Button::new("cancel-remove-login-item", "Cancel")
                        .disabled(self.login_item_busy.is_some())
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| {
                                settings.login_item_remove = None;
                                cx.notify();
                            });
                        }),
                )
                .child(
                    Button::new("confirm-remove-login-item", "Move to Trash")
                        .primary()
                        .busy(self.login_item_busy.as_deref() == Some("remove"))
                        .disabled(self.login_item_busy.is_some())
                        .on_click(move |_, _, cx| {
                            confirm_view.update(cx, |settings, cx| {
                                settings.confirm_remove_login_item(cx);
                            });
                        }),
                )
                .into_any_element()]));
        }
    }
}
