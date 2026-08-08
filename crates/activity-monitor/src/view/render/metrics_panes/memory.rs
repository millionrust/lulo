//! System Monitor memory-pressure projection.

use super::*;

impl MonitorView {
    pub(super) fn render_mem_pressure(&self) -> impl IntoElement {
        let fraction = (self.sampler.aggregates.mem_used as f32
            / self.sampler.aggregates.mem_total as f32)
            .clamp(0.0, 1.0);
        let (color, label): (gpui::Hsla, &str) = if fraction < 0.60 {
            (gpui::rgb(0x28b463).into(), "Normal")
        } else if fraction < 0.80 {
            (gpui::rgb(0xff9500).into(), "Elevated")
        } else {
            (gpui::rgb(0xff3b30).into(), "High")
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
                    .rounded(px(5.0))
                    .bg(mac::chrome())
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(fraction))
                            .rounded(px(5.0))
                            .bg(color),
                    ),
            )
    }
}
