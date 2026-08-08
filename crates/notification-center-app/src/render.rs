mod history;

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
