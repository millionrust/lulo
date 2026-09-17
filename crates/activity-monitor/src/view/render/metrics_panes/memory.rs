//! System Monitor memory-pressure projection.

use super::*;

impl MonitorView {
    pub(super) fn render_mem_pressure(&self) -> impl IntoElement {
        let fraction = (self.sampler.aggregates.mem_used as f32
            / self.sampler.aggregates.mem_total as f32)
            .clamp(0.0, 1.0);
        let (color, label): (gpui::Hsla, &str) = if fraction < 0.60 {
            (mac::system_green(), "Normal")
        } else if fraction < 0.80 {
            (mac::system_orange(), "Elevated")
        } else {
            (mac::system_red(), "High")
        };
        div()
            .v_flex()
            .gap_2()
            .pt_1()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text_tertiary())
                            .child("MEMORY PRESSURE"),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(color)
                            .child(label),
                    ),
            )
            .child(
                div()
                    .h(px(10.0))
                    .w_full()
                    .rounded(px(mac::radius_menu_item()))
                    .bg(mac::chrome())
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(fraction))
                            .rounded(px(mac::radius_menu_item()))
                            .bg(color),
                    ),
            )
    }
}
