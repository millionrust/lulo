//! System Settings display arrangement well.

use super::*;

/// macOS 26 opens Displays with a full-width darker well (≈160 pt) under the
/// toolbar showing the displays as pictures with their names; rmac draws
/// each enabled output to scale from its niri logical geometry.
const WELL_HEIGHT: f32 = 160.0;
const ARRANGEMENT_WIDTH: f32 = 300.0;
const ARRANGEMENT_HEIGHT: f32 = 96.0;

pub(in crate::controller) fn display_layout_preview(outputs: &[rmac_display::Output]) -> Div {
    let enabled = outputs
        .iter()
        .filter_map(|output| Some((output, output.logical.as_ref()?)))
        .collect::<Vec<_>>();
    let well = div()
        .mx(px(-style::DETAIL_INSET))
        .mt(px(-style::FIRST_SECTION_TOP))
        .mb(px(style::DETAIL_INSET))
        .h(px(WELL_HEIGHT))
        .v_flex()
        .items_center()
        .justify_center()
        .gap(px(10.0))
        .bg(style::well_fill())
        .border_b_1()
        .border_color(sep());
    let Some(min_x) = enabled.iter().map(|(_, logical)| logical.x).min() else {
        return well.child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child("No enabled display layout is available."),
        );
    };
    let min_y = enabled
        .iter()
        .map(|(_, logical)| logical.y)
        .min()
        .unwrap_or_default();
    let max_x = enabled
        .iter()
        .map(|(_, logical)| i64::from(logical.x) + i64::from(logical.width))
        .max()
        .unwrap_or(1);
    let max_y = enabled
        .iter()
        .map(|(_, logical)| i64::from(logical.y) + i64::from(logical.height))
        .max()
        .unwrap_or(1);
    let span_x = (max_x - i64::from(min_x)).max(1) as f32;
    let span_y = (max_y - i64::from(min_y)).max(1) as f32;
    let scale = (ARRANGEMENT_WIDTH / span_x).min(ARRANGEMENT_HEIGHT / span_y);
    let single = enabled.len() == 1;
    let mut canvas = div().relative().w(px(span_x * scale)).h(px(span_y * scale));
    let mut caption: Option<String> = None;
    for (output, logical) in enabled {
        let left = (logical.x - min_x) as f32 * scale;
        let top = (logical.y - min_y) as f32 * scale;
        let width = (logical.width as f32 * scale).max(24.0);
        let height = (logical.height as f32 * scale).max(18.0);
        if single {
            caption = Some(output.detail.clone().unwrap_or_else(|| output.name.clone()));
        }
        canvas = canvas.child(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width))
                .h(px(height))
                .flex()
                .items_center()
                .justify_center()
                .px_1()
                .rounded(px(4.0))
                .border_2()
                .border_color(if output.primary {
                    accent()
                } else {
                    rmac_ui::mac::text_tertiary()
                })
                .bg(gpui::linear_gradient(
                    160.0,
                    gpui::linear_color_stop(hsl(0x3b5ea6), 0.0),
                    gpui::linear_color_stop(hsl(0x1f3a5a), 1.0),
                ))
                .text_size(rmac_ui::text_px(11.0))
                .text_color(hsl(0xffffff))
                .overflow_hidden()
                .when(!single, |display| display.child(output.name.clone())),
        );
    }
    well.child(canvas).when_some(caption, |well, caption| {
        well.child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .font_weight(rmac_ui::mac::BOLD)
                .text_color(label())
                .child(caption),
        )
    })
}
