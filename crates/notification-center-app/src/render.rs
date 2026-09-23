//! Notification Center as macOS 26 draws it: no panel, heading or footer.
//! Cards float in a column under the menu bar over a soft backdrop dim, and
//! an empty Center reads "No recent notifications" above "Edit Widgets".
//! Every number comes from design-lab/notifications.html.

mod history;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    canvas, div, img, linear_color_stop, linear_gradient, px, rgba, size, AnyElement, Context,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_component::{IconName, StyledExt as _};
use rmac_notification_center_app::accessibility::{
    EDIT_WIDGETS_LABEL, EMPTY_TITLE, SHOW_LESS_LABEL, UNAVAILABLE_TITLE,
};
use rmac_notifications::NotificationId;
use rmac_notifications_linux::center::{ActionSelection, HistoryRecord};
use rmac_ui::{mac, Button, ButtonRole};

use crate::model::{
    panel_height, relative_time, ApplicationIdentity, Busy, RecordGroup, COLUMN_BOTTOM, COLUMN_TOP,
    DIM_HEIGHT, PANEL_WIDTH,
};
use crate::view::NotificationCenterView;
use history::{card, Clock};

/// The column's right inset; the × on a card overhangs its left edge, so the
/// scroll area is widened by the same amount to keep it unclipped.
const COLUMN_RIGHT: f32 = 8.0;
const COLUMN_LEFT_ROOM: f32 = 8.0;

/// Measured backdrop: × 0.87 luminance, full for 40 pt, gone at 182 pt, with
/// a 95 pt feather on its left edge.
const DIM: u32 = 0x00000021;
const DIM_FULL: f32 = 40.0;
const DIM_FEATHER: f32 = 95.0;

/// Measured empty state (text box tops from the bar, centres from the right).
const EMPTY_TITLE_TOP: f32 = 45.0;
const EMPTY_TITLE_CENTRE: f32 = 180.0;
const EMPTY_TITLE_COLOUR: u32 = 0xDCDCDCFF;
const EDIT_TOP: f32 = 96.5;
const EDIT_CENTRE: f32 = 188.0;
const EDIT_WIDTH: f32 = 85.5;
const EDIT_HEIGHT: f32 = 23.0;
const EDIT_FILL: u32 = 0xFFFFFF25;
const EDIT_BORDER: u32 = 0xFFFFFF45;
const EDIT_TEXT: u32 = 0xFFFFFF94;

fn backdrop() -> impl IntoElement {
    let fade = |top: f32| {
        linear_gradient(
            180.0,
            linear_color_stop(rgba(DIM), top),
            linear_color_stop(rgba(DIM & 0xFFFF_FF00), 1.0),
        )
    };
    div()
        .absolute()
        .top_0()
        .right_0()
        .w(px(PANEL_WIDTH))
        .h(px(DIM_HEIGHT))
        // Full-strength region right of the feather.
        .child(
            div()
                .absolute()
                .top_0()
                .right_0()
                .left(px(DIM_FEATHER))
                .bottom_0()
                .bg(fade(DIM_FULL / DIM_HEIGHT)),
        )
        // Left feather, fading in horizontally.
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .w(px(DIM_FEATHER))
                .h(px(DIM_FULL * 2.0))
                .bg(linear_gradient(
                    90.0,
                    linear_color_stop(rgba(DIM & 0xFFFF_FF00), 0.0),
                    linear_color_stop(rgba(DIM), 1.0),
                )),
        )
}

fn empty_state(title: &'static str, show_edit: bool) -> impl IntoElement {
    div()
        .absolute()
        .top_0()
        .right_0()
        .w(px(PANEL_WIDTH))
        .h(px(DIM_HEIGHT))
        .child(
            div()
                .absolute()
                .top(px(EMPTY_TITLE_TOP))
                .right_0()
                .w(px(EMPTY_TITLE_CENTRE * 2.0))
                .flex()
                .justify_center()
                .text_size(rmac_ui::text_px(15.0))
                .line_height(px(20.0))
                .font_weight(mac::SEMIBOLD)
                .text_color(rgba(EMPTY_TITLE_COLOUR))
                .child(title),
        )
        .when(show_edit, |state| {
            // There are no widgets yet, so the pill is shown as on the Mac
            // but has nothing to edit.
            state.child(
                div()
                    .absolute()
                    .top(px(EDIT_TOP))
                    .right(px(EDIT_CENTRE - EDIT_WIDTH / 2.0))
                    .w(px(EDIT_WIDTH))
                    .h(px(EDIT_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(rgba(EDIT_FILL))
                    .border_1()
                    .border_color(rgba(EDIT_BORDER))
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(rgba(EDIT_TEXT))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(EDIT_WIDGETS_LABEL),
            )
        })
}

fn notice(message: SharedString) -> impl IntoElement {
    div()
        .w(px(card::WIDTH))
        .px(px(card::PAD_LEFT))
        .py(px(12.0))
        .rounded(px(card::RADIUS))
        .bg(rgba(card::FILL))
        .border_1()
        .border_color(rgba(card::BORDER))
        .text_size(rmac_ui::text_px(12.0))
        .line_height(px(card::LINE))
        .text_color(rgba(card::BODY))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(message)
}

impl Render for NotificationCenterView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let clock = Clock::now();
        let groups = self.groups();
        let has_records = !groups.is_empty();
        let unavailable = self.snapshot.is_none() && self.stream_error.is_some();
        let empty = self.snapshot.is_some() && !has_records;
        let group_elements = groups
            .iter()
            .map(|group| self.group(group, clock, cx))
            .collect::<Vec<_>>();
        let notices = [self.stream_error.clone(), self.operation_error.clone()]
            .into_iter()
            .flatten()
            .filter(|_| !unavailable)
            .map(notice)
            .collect::<Vec<_>>();
        let dismiss_view = cx.entity();

        // Shrinks (or grows) the layer surface to the column's natural
        // height after layout, as macOS sizes Notification Center to its cards.
        let measure = canvas(
            |bounds, window, cx| {
                let wanted = panel_height(f32::from(bounds.size.height));
                let current = f32::from(window.viewport_size().height);
                if (wanted - current).abs() > 0.5 {
                    window.defer(cx, move |window, _| {
                        window.resize(size(px(PANEL_WIDTH), px(wanted)));
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();

        div()
            .size_full()
            .relative()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.dismiss(window, cx);
                }
            }))
            // A click on the backdrop, outside every card, closes the Center.
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                dismiss_view.update(cx, |this, cx| this.dismiss(window, cx));
            })
            .text_color(mac::text())
            .child(backdrop())
            .when(empty, |root| root.child(empty_state(EMPTY_TITLE, true)))
            .when(unavailable, |root| {
                root.child(empty_state(UNAVAILABLE_TITLE, false))
            })
            .child(
                div()
                    .id("notification-center-scroll")
                    .absolute()
                    .top_0()
                    .right(px(COLUMN_RIGHT))
                    .w(px(card::WIDTH + COLUMN_LEFT_ROOM))
                    .max_h(px(panel_height(f32::MAX)))
                    .overflow_y_scroll()
                    .pt(px(COLUMN_TOP))
                    .pb(px(COLUMN_BOTTOM))
                    .pl(px(COLUMN_LEFT_ROOM))
                    .child(
                        div()
                            .relative()
                            .w(px(card::WIDTH))
                            .v_flex()
                            .gap(px(card::GAP))
                            .children(notices)
                            .children(group_elements)
                            .child(measure),
                    ),
            )
    }
}
