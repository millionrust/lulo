//! System Settings display arrangement preview projection.

use super::*;

pub(in crate::controller) fn display_layout_preview(outputs: &[rmac_display::Output]) -> Div {
    let enabled = outputs
        .iter()
        .filter_map(|output| Some((output, output.logical.as_ref()?)))
        .collect::<Vec<_>>();
    let Some(min_x) = enabled.iter().map(|(_, logical)| logical.x).min() else {
        return note_card("No enabled display layout is available.");
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
    let scale = (440.0 / span_x).min(130.0 / span_y);
    let mut canvas = div()
        .relative()
        .w_full()
        .h(px(150.0))
        .rounded(px(mac::radius_menu()))
        .bg(rmac_ui::mac::control_fill())
        .border_1()
        .border_color(sep())
        .overflow_hidden();
    for (index, (output, logical)) in enabled.into_iter().enumerate() {
        let left = 10.0 + (logical.x - min_x) as f32 * scale;
        let top = 10.0 + (logical.y - min_y) as f32 * scale;
        let width = (logical.width as f32 * scale).max(24.0);
        let height = (logical.height as f32 * scale).max(18.0);
        let title = if output.primary {
            format!("{} · Main", index + 1)
        } else {
            format!("{} · {}", index + 1, output.name)
        };
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
                .rounded(px(mac::radius_menu_item()))
                .border_2()
                .border_color(if output.primary { accent() } else { sep() })
                .bg(if output.primary { accent() } else { card_bg() })
                .text_size(rmac_ui::text_px(11.0))
                .text_color(if output.primary {
                    hsl(0xffffff)
                } else {
                    label()
                })
                .overflow_hidden()
                .child(title),
        );
    }
    div()
        .p_2()
        .rounded(px(mac::radius_menu()))
        .bg(card_bg())
        .border_1()
        .border_color(sep())
        .child(canvas)
}
