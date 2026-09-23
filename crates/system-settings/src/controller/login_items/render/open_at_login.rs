//! Open at Login: the Mac's Item | Kind table with its +/− bar.

use super::*;

/// The Kind column starts 230 into the table, as on the Mac.
const KIND_COLUMN_X: f32 = 230.0;

impl Settings {
    pub(super) fn append_open_at_login(
        &self,
        view: Entity<Self>,
        snapshot: &rmac_login_items::Snapshot,
        cards: &mut Vec<Div>,
    ) {
        let busy_any = self.login_item_busy.is_some();
        cards.push(section_with_note(
            "Open at Login",
            "These items will open automatically when you log in.",
            true,
        ));

        let rows = snapshot
            .items
            .iter()
            .map(|item| {
                let id = item.id.clone();
                let toggle_view = view.clone();
                let reveal_view = view.clone();
                let reveal_id = item.id.clone();
                let remove_view = view.clone();
                let remove_id = item.id.clone();
                let busy = self.login_item_busy.as_deref() == Some(item.id.as_str());
                let can_toggle = item.can_toggle && !busy_any;
                let enabled = item.enabled;
                let removable = item.user_owned && !item.managed_override;
                let kind = item.session_detail.clone().unwrap_or_else(|| {
                    if item.user_owned {
                        "User entry".into()
                    } else {
                        "System entry".into()
                    }
                });
                let content = div()
                    .flex()
                    .items_center()
                    .w_full()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id(SharedString::from(format!("login-item-{}", item.id)))
                            .when(can_toggle, |check| {
                                check.cursor_pointer().on_click(move |_, _, cx| {
                                    toggle_view.update(cx, |settings, cx| {
                                        settings.set_login_item_enabled(id.clone(), !enabled, cx);
                                    });
                                })
                            })
                            .when(!can_toggle, |check| check.opacity(0.5))
                            .child(form_checkbox(enabled)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(label())
                            .child(item.name.clone()),
                    )
                    .child(
                        div()
                            .w(px(KIND_COLUMN_X - 2.0 * style::ROW_PADDING - 20.0))
                            .flex_none()
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(label())
                            .child(if busy { "Saving…".to_string() } else { kind }),
                    )
                    .child(reveal_button(
                        SharedString::from(format!("reveal-login-item-{}", item.id)),
                        move |_, cx| {
                            reveal_view.update(cx, |settings, cx| {
                                settings.reveal_login_item(reveal_id.clone(), false, cx);
                            });
                        },
                    ))
                    .when(removable, |row| {
                        row.child(
                            div()
                                .id(SharedString::from(format!("remove-login-item-{}", item.id)))
                                .w(px(style::INFO_BUTTON))
                                .flex_none()
                                .text_center()
                                .text_size(rmac_ui::text_px(13.0))
                                .text_color(label())
                                .cursor_pointer()
                                .on_click(move |_, _, cx| {
                                    remove_view.update(cx, |settings, cx| {
                                        settings.request_remove_login_item(remove_id.clone(), cx);
                                    });
                                })
                                .child("−"),
                        )
                    });
                well_row(
                    SharedString::from(format!("login-item-row-{}", item.id)),
                    false,
                    content,
                    None,
                )
            })
            .collect::<Vec<_>>();

        let header = div()
            .flex()
            .w_full()
            .child(div().flex_1().pl(px(24.0)).child("Item"))
            .child(
                div()
                    .w(px(KIND_COLUMN_X - 2.0 * style::ROW_PADDING - 20.0))
                    .flex_none()
                    .child("Kind"),
            )
            .child(div().w(px(2.0 * style::INFO_BUTTON + 8.0)).flex_none())
            .into_any_element();
        let choose_view = view.clone();
        let add: Option<FormHandler> = (!busy_any).then(|| {
            Rc::new(move |_: &mut Window, cx: &mut App| {
                choose_view.update(cx, |settings, cx| settings.choose_login_item(cx));
            }) as FormHandler
        });
        cards.push(well(Some(header), rows, 3, add, None));

        if let Some(preview) = &self.login_item_add {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(footnote(format!(
                "“{}” ({}) will be allowed to run at login. {}",
                preview.name,
                preview.id,
                if preview.replacing {
                    "It replaces your existing entry with the same name."
                } else {
                    "A copy is placed in your autostart folder."
                }
            )));
            cards.push(card(vec![
                value_button_row(
                    "Command at login",
                    Some(preview.command.clone().into()),
                    None,
                    None,
                ),
                button_row(vec![
                    push_button("cancel-add-login-item", "Cancel")
                        .disabled(busy_any)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| {
                                settings.login_item_add = None;
                                cx.notify();
                            });
                        })
                        .into_any_element(),
                    Button::new(
                        "confirm-add-login-item",
                        if preview.replacing { "Replace" } else { "Add" },
                    )
                    .primary()
                    .h(px(24.0))
                    .busy(self.login_item_busy.as_deref() == Some("add"))
                    .disabled(busy_any)
                    .on_click(move |_, _, cx| {
                        confirm_view.update(cx, |settings, cx| settings.confirm_add_login_item(cx));
                    })
                    .into_any_element(),
                ]),
            ]));
        }
        if let Some(preview) = &self.login_item_remove {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(footnote(format!(
                "Remove “{}”? Its entry moves to the Bin; the application itself is not deleted. A system entry with the same name stays listed but off.",
                preview.name
            )));
            cards.push(footer_buttons(vec![
                push_button("cancel-remove-login-item", "Cancel")
                    .disabled(busy_any)
                    .on_click(move |_, _, cx| {
                        cancel_view.update(cx, |settings, cx| {
                            settings.login_item_remove = None;
                            cx.notify();
                        });
                    })
                    .into_any_element(),
                Button::new("confirm-remove-login-item", "Move to Bin")
                    .primary()
                    .h(px(24.0))
                    .busy(self.login_item_busy.as_deref() == Some("remove"))
                    .disabled(busy_any)
                    .on_click(move |_, _, cx| {
                        confirm_view.update(cx, |settings, cx| {
                            settings.confirm_remove_login_item(cx);
                        });
                    })
                    .into_any_element(),
            ]));
        }
    }
}
