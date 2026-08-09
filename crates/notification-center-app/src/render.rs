mod history;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, img, px, AnyElement, Context, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_component::{IconName, StyledExt as _};
use rmac_notification_center_app::accessibility::{
    notification_count_label, CLEAR_ALL_LABEL, CLEAR_LABEL, EMPTY_MESSAGE, EMPTY_TITLE,
    LOADING_LABEL, PANEL_TITLE, REFRESH_LABEL, SETTINGS_LABEL, UNAVAILABLE_MESSAGE,
    UNAVAILABLE_TITLE, URGENT_LABEL,
};
use rmac_notifications::Priority;
use rmac_notifications_linux::center::{ActionSelection, HistoryRecord};
use rmac_ui::{mac, Button, ButtonRole, EmptyState, Progress};

use crate::model::{ApplicationIdentity, Busy, RecordGroup};
use crate::view::NotificationCenterView;

impl Render for NotificationCenterView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .text_color(mac::text())
            .child(
                div()
                    .h(px(44.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text())
                            .child(PANEL_TITLE),
                    )
                    .when(has_records, |header| {
                        header.child(
                            Button::new("clear-all-notifications", "")
                                .icon(IconName::Close)
                                .tooltip(CLEAR_ALL_LABEL)
                                .role(ButtonRole::Ghost)
                                .xsmall()
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
                        .rounded(px(mac::radius_control()))
                        .bg(mac::material())
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
                        .rounded(px(mac::radius_control()))
                        .bg(mac::material())
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
                    .px_2()
                    .pt_0p5()
                    .pb_2()
                    .when(self.snapshot.is_none() && !unavailable, |content| {
                        content.child(
                            div().h_full().flex().items_center().justify_center().child(
                                div()
                                    .w_full()
                                    .p_4()
                                    .rounded(px(mac::radius_popover()))
                                    .border_1()
                                    .border_color(mac::separator())
                                    .bg(mac::material())
                                    .child(Progress::indeterminate().label(LOADING_LABEL)),
                            ),
                        )
                    })
                    .when(unavailable, |content| {
                        content.child(
                            div().h_full().flex().items_center().justify_center().child(
                                div()
                                    .w_full()
                                    .p_4()
                                    .rounded(px(mac::radius_popover()))
                                    .border_1()
                                    .border_color(mac::separator())
                                    .bg(mac::material())
                                    .child(
                                        EmptyState::new(UNAVAILABLE_TITLE)
                                            .message(UNAVAILABLE_MESSAGE),
                                    ),
                            ),
                        )
                    })
                    .when(self.snapshot.is_some() && !has_records, |content| {
                        content.child(
                            div().h_full().flex().items_center().justify_center().child(
                                div()
                                    .w_full()
                                    .p_4()
                                    .rounded(px(mac::radius_popover()))
                                    .border_1()
                                    .border_color(mac::separator())
                                    .bg(mac::material())
                                    .child(EmptyState::new(EMPTY_TITLE).message(EMPTY_MESSAGE)),
                            ),
                        )
                    })
                    .when(has_records, |content| {
                        content.v_flex().gap_2().children(group_elements)
                    }),
            )
            .child(
                div()
                    .h(px(40.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .px_2()
                    .child(
                        Button::new("refresh-notifications", "")
                            .icon(IconName::Redo)
                            .tooltip(REFRESH_LABEL)
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
