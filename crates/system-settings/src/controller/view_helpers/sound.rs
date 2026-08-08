//! System Settings Sound level and balance slider row projection.

use super::*;

pub(in crate::controller) fn slider_row(
    title: &'static str,
    state: &Entity<SliderState>,
    value: SharedString,
) -> Div {
    row_base()
        .child(
            div()
                .w(px(110.0))
                .flex_none()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child(title),
        )
        .child(div().flex_1().child(Slider::new(state).w_full()))
        .child(
            div()
                .w(px(44.0))
                .flex_none()
                .text_right()
                .text_size(rmac_ui::text_px(12.0))
                .text_color(secondary())
                .child(value),
        )
}

pub(in crate::controller) fn balance_slider_row(state: &Entity<SliderState>) -> Div {
    row_base()
        .child(
            div()
                .w(px(110.0))
                .flex_none()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child("Balance"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .flex_1()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(secondary())
                        .child("L"),
                )
                .child(div().flex_1().child(Slider::new(state).w_full()))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(secondary())
                        .child("R"),
                ),
        )
        .child(div().w(px(44.0)).flex_none())
}
