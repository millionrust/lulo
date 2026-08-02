use chrono::Local;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, img, px, AnyElement, Context, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_component::StyledExt as _;
use rmac_notification_center_app::accessibility::{
    notification_count_label, CLEAR_ALL_LABEL, CLEAR_LABEL, EMPTY_MESSAGE, EMPTY_TITLE,
    LOADING_LABEL, PANEL_TITLE, REFRESH_LABEL, SETTINGS_LABEL, TURN_OFF_LABEL, UNAVAILABLE_MESSAGE,
    UNAVAILABLE_TITLE, URGENT_LABEL,
};
use rmac_notifications::Priority;
use rmac_notifications_linux::center::{ActionSelection, HistoryRecord};
use rmac_ui::{mac, Button, ButtonRole, EmptyState, Progress};

use crate::model::{ApplicationIdentity, Busy, RecordGroup};
use crate::view::NotificationCenterView;

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

    fn group(&self, group: &RecordGroup<'_>, index: usize, cx: &Context<Self>) -> AnyElement {
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

impl Render for NotificationCenterView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Local::now();
        let groups = self.groups();
        let has_records = !groups.is_empty();
        let unavailable = self.snapshot.is_none() && self.stream_error.is_some();
        let group_elements = groups
            .iter()
            .enumerate()
            .map(|(index, group)| self.group(group, index, cx))
            .collect::<Vec<_>>();
        let view = cx.entity();
        let clear_view = view.clone();
        let refresh_view = view.clone();
        let settings_view = view.clone();
        let busy = self.busy.is_some();

        div()
            .size_full()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.dismiss(window, cx);
                }
            }))
            .v_flex()
            .overflow_hidden()
            .rounded(px(18.0))
            .border_1()
            .border_color(mac::separator())
            .shadow_xl()
            .bg(mac::window())
            .text_color(mac::text())
            .child(
                div()
                    .flex_none()
                    .v_flex()
                    .gap_0p5()
                    .px_5()
                    .pt_4()
                    .pb_3()
                    .border_b_1()
                    .border_color(mac::separator())
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(27.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text())
                            .child(now.format("%H:%M").to_string()),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(mac::text_secondary())
                            .child(now.format("%A, %B %-d").to_string()),
                    ),
            )
            .child(
                div()
                    .h(px(52.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(mac::SEMIBOLD)
                            .child(PANEL_TITLE),
                    )
                    .when(has_records, |header| {
                        header.child(
                            Button::new("clear-all-notifications", CLEAR_ALL_LABEL)
                                .role(ButtonRole::Ghost)
                                .disabled(busy)
                                .busy(matches!(self.busy, Some(Busy::ClearAll)))
                                .on_click(move |_, _, cx| {
                                    clear_view.update(cx, |this, cx| this.clear(None, cx));
                                }),
                        )
                    }),
            )
            .when_some(self.stream_error.clone(), |panel, error| {
                panel.child(
                    div()
                        .mx_4()
                        .mb_2()
                        .px_3()
                        .py_2()
                        .rounded(px(9.0))
                        .bg(mac::warning_background())
                        .border_1()
                        .border_color(mac::warning_border())
                        .text_size(rmac_ui::text_px(11.0))
                        .text_color(mac::warning_text())
                        .child(error),
                )
            })
            .when_some(self.operation_error.clone(), |panel, error| {
                panel.child(
                    div()
                        .mx_4()
                        .mb_2()
                        .px_3()
                        .py_2()
                        .rounded(px(9.0))
                        .bg(mac::error_background())
                        .border_1()
                        .border_color(mac::error_border())
                        .text_size(rmac_ui::text_px(11.0))
                        .text_color(mac::danger())
                        .child(error),
                )
            })
            .child(
                div()
                    .id("notification-center-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_3()
                    .pt_1()
                    .pb_3()
                    .when(self.snapshot.is_none() && !unavailable, |content| {
                        content.child(
                            div()
                                .h_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(Progress::indeterminate().label(LOADING_LABEL)),
                        )
                    })
                    .when(unavailable, |content| {
                        content.child(
                            div().h_full().flex().items_center().justify_center().child(
                                EmptyState::new(UNAVAILABLE_TITLE).message(UNAVAILABLE_MESSAGE),
                            ),
                        )
                    })
                    .when(self.snapshot.is_some() && !has_records, |content| {
                        content.child(
                            div()
                                .h_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(EmptyState::new(EMPTY_TITLE).message(EMPTY_MESSAGE)),
                        )
                    })
                    .when(has_records, |content| {
                        content.v_flex().gap_3().children(group_elements)
                    }),
            )
            .child(
                div()
                    .h(px(48.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .border_t_1()
                    .border_color(mac::separator())
                    .child(
                        Button::new("refresh-notifications", REFRESH_LABEL)
                            .ghost()
                            .xsmall()
                            .on_click(move |_, _, cx| {
                                refresh_view.update(cx, |this, cx| this.refresh(cx));
                            }),
                    )
                    .child(
                        Button::new("open-notification-settings", SETTINGS_LABEL)
                            .ghost()
                            .xsmall()
                            .on_click(move |_, _, cx| {
                                settings_view.update(cx, |this, cx| this.open_settings(cx));
                            }),
                    ),
            )
    }
}
