//! Notification Center application icon, record, and grouped-history projection.

use super::*;

impl NotificationCenterView {
    fn app_icon(identity: &ApplicationIdentity, size: f32) -> AnyElement {
        if let Some(icon) = &identity.icon {
            return img(icon.clone())
                .size(px(size))
                .rounded(px(size * 0.22))
                .into_any_element();
        }
        let initial = identity
            .name
            .chars()
            .next()
            .map(|character| character.to_uppercase().to_string())
            .unwrap_or_else(|| "•".into());
        div()
            .size(px(size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(size * 0.22))
            .bg(mac::control_fill())
            .text_color(mac::text_secondary())
            .font_weight(mac::SEMIBOLD)
            .text_size(rmac_ui::text_px(size * 0.4))
            .child(initial)
            .into_any_element()
    }

    fn record(&self, record: &HistoryRecord, index: usize, cx: &Context<Self>) -> AnyElement {
        let urgent = record.priority == Priority::Urgent;
        let action_buttons = record
            .actions
            .iter()
            .enumerate()
            .map(|(action_index, action)| {
                let id = record.id;
                let selection = action.selection;
                let view = cx.entity();
                let action_busy = matches!(
                    &self.busy,
                    Some(Busy::Invoke(candidate, candidate_selection))
                        if *candidate == id && *candidate_selection == selection
                );
                Button::new(
                    SharedString::from(format!(
                        "notification-{}-action-{}",
                        record.id.get(),
                        action_index
                    )),
                    action.label.clone(),
                )
                .xsmall()
                .role(if selection == ActionSelection::Default {
                    ButtonRole::Primary
                } else {
                    ButtonRole::Secondary
                })
                .disabled(self.busy.is_some())
                .busy(action_busy)
                .on_click(move |_, _, cx| {
                    view.update(cx, |this, cx| this.invoke_action(id, selection, cx));
                })
            })
            .collect::<Vec<_>>();
        div()
            .id(SharedString::from(format!(
                "notification-{}-{}",
                record.id.get(),
                index
            )))
            .w_full()
            .flex()
            .items_start()
            .gap_2()
            .px_3()
            .py_2()
            .when(index > 0, |row| {
                row.border_t_1().border_color(mac::separator())
            })
            .child(
                div()
                    .mt(px(6.0))
                    .size(px(6.0))
                    .flex_none()
                    .rounded_full()
                    .bg(if record.unread {
                        mac::accent()
                    } else {
                        gpui::transparent_black()
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(rmac_ui::text_px(13.0))
                                    .font_weight(mac::SEMIBOLD)
                                    .text_color(mac::text())
                                    .line_clamp(2)
                                    .child(record.content.title().to_owned()),
                            )
                            .when(urgent, |title| {
                                title.child(
                                    div()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded_full()
                                        .bg(mac::error_background())
                                        .text_color(mac::danger())
                                        .text_size(rmac_ui::text_px(9.0))
                                        .font_weight(mac::SEMIBOLD)
                                        .child(URGENT_LABEL),
                                )
                            }),
                    )
                    .when(!record.content.body().is_empty(), |content| {
                        content.child(
                            div()
                                .whitespace_normal()
                                .line_clamp(3)
                                .text_size(rmac_ui::text_px(12.0))
                                .line_height(gpui::relative(1.35))
                                .text_color(mac::text_secondary())
                                .child(record.content.body().to_owned()),
                        )
                    })
                    .when(!action_buttons.is_empty(), |content| {
                        content.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_1()
                                .pt_1()
                                .children(action_buttons),
                        )
                    }),
            )
            .into_any_element()
    }

    pub(super) fn group(
        &self,
        group: &RecordGroup<'_>,
        index: usize,
        cx: &Context<Self>,
    ) -> AnyElement {
        let identity = self.identity(group.app_id);
        let clear_id = group.app_id.to_owned();
        let disable_id = group.app_id.to_owned();
        let view = cx.entity();
        let clear_view = view.clone();
        let disable_view = view.clone();
        let busy = self.busy.is_some();
        let clear_busy = matches!(&self.busy, Some(Busy::ClearApp(id)) if id == group.app_id);
        let disable_busy = matches!(&self.busy, Some(Busy::DisableApp(id)) if id == group.app_id);
        let records = group
            .records
            .iter()
            .enumerate()
            .map(|(index, record)| self.record(record, index, cx))
            .collect::<Vec<_>>();

        div()
            .w_full()
            .v_flex()
            .overflow_hidden()
            .rounded(px(14.0))
            .border_1()
            .border_color(mac::separator())
            .bg(mac::raised())
            .shadow_sm()
            .child(
                div()
                    .h(px(48.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .border_b_1()
                    .border_color(mac::separator())
                    .child(Self::app_icon(&identity, 28.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rmac_ui::text_px(12.0))
                                    .font_weight(mac::SEMIBOLD)
                                    .text_color(mac::text())
                                    .child(identity.name),
                            )
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(10.0))
                                    .text_color(mac::text_tertiary())
                                    .child(notification_count_label(group.records.len())),
                            ),
                    )
                    .when(self.policy_enabled(group.app_id), |header| {
                        header.child(
                            Button::new(
                                SharedString::from(format!("disable-group-{index}")),
                                TURN_OFF_LABEL,
                            )
                            .ghost()
                            .xsmall()
                            .disabled(busy)
                            .busy(disable_busy)
                            .on_click(move |_, _, cx| {
                                disable_view.update(cx, |this, cx| {
                                    this.disable_app(disable_id.clone(), cx)
                                });
                            }),
                        )
                    })
                    .child(
                        Button::new(
                            SharedString::from(format!("clear-group-{index}")),
                            CLEAR_LABEL,
                        )
                        .ghost()
                        .xsmall()
                        .disabled(busy)
                        .busy(clear_busy)
                        .on_click(move |_, _, cx| {
                            clear_view
                                .update(cx, |this, cx| this.clear(Some(clear_id.clone()), cx));
                        }),
                    ),
            )
            .children(records)
            .into_any_element()
    }
}
