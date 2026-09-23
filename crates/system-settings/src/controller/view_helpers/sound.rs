//! System Settings Sound level and balance slider row projection.

use super::*;

/// macOS 26 volume rows: the label, then a 230 pt control of a small
/// speaker, the slider and a loud speaker, right-aligned. The value is
/// exposed as the slider's tooltip text rather than a trailing number.
pub(in crate::controller) fn slider_row(
    title: &'static str,
    state: &Entity<SliderState>,
    value: SharedString,
) -> Div {
    row_base().child(text_block(title.into(), None)).child(
        div()
            .id(SharedString::from(format!("slider-{title}")))
            .w(px(SLIDER_CONTROL_WIDTH))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.0))
            .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(value.clone()).build(window, cx)
            })
            .child(glyph("icons/volume-2.svg", 11.0, secondary()))
            .child(div().flex_1().child(Slider::new(state).w_full()))
            .child(glyph("icons/volume-2.svg", 15.0, secondary())),
    )
}

const SLIDER_CONTROL_WIDTH: f32 = 230.0;

pub(in crate::controller) fn balance_slider_row(state: &Entity<SliderState>) -> Div {
    // macOS 26: "Left" and "Right" in 11 pt under the ends of the slider.
    row_base()
        .items_start()
        .child(text_block("Balance".into(), None))
        .child(
            div()
                .w(px(SLIDER_CONTROL_WIDTH))
                .flex_none()
                .v_flex()
                .gap(px(2.0))
                .child(Slider::new(state).w_full())
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .text_size(rmac_ui::text_px(11.0))
                        .line_height(px(14.0))
                        .text_color(label())
                        .child("Left")
                        .child("Right"),
                ),
        )
}
