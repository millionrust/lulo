//! System Monitor system-wide network-interface table projection.

use super::*;

impl MonitorView {
    /// Network is system-wide because the current authority has no reliable
    /// per-process network accounting.
    pub(in crate::view::render) fn render_network_pane(&self) -> impl IntoElement {
        let teal = mac::system_teal();
        let figure = |value: String, color: gpui::Hsla| {
            div()
                .w(px(110.0))
                .text_size(rmac_ui::text_px(12.0))
                .text_color(color)
                .text_right()
                .child(value)
        };

        let header = div()
            .h_flex()
            .items_center()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(mac::separator())
            .child(
                div()
                    .flex_1()
                    .text_size(rmac_ui::text_px(11.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text_tertiary())
                    .child("INTERFACE"),
            )
            .children(
                ["RCVD", "SENT", "↓ RATE", "↑ RATE"]
                    .into_iter()
                    .map(|label| {
                        div()
                            .w(px(110.0))
                            .text_size(rmac_ui::text_px(11.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text_tertiary())
                            .text_right()
                            .child(label)
                    }),
            );

        let rows: Vec<gpui::AnyElement> = self
            .sampler
            .interfaces
            .iter()
            .enumerate()
            .map(|(index, interface)| {
                let active = interface.recv_rate + interface.sent_rate > 0.0;
                div()
                    .h_flex()
                    .items_center()
                    .px_3()
                    .py_1p5()
                    .when(index % 2 == 1, |element| element.bg(mac::hover()))
                    .child(
                        div()
                            .flex_1()
                            .h_flex()
                            .items_center()
                            .gap_2()
                            .child(div().size(px(7.0)).rounded_full().bg(if active {
                                teal.into()
                            } else {
                                mac::text_tertiary()
                            }))
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(13.0))
                                    .text_color(mac::text())
                                    .child(interface.name.clone()),
                            ),
                    )
                    .child(figure(format_bytes(interface.total_recv), mac::text()))
                    .child(figure(format_bytes(interface.total_sent), mac::text()))
                    .child(figure(
                        format_rate(interface.recv_rate),
                        if active {
                            teal.into()
                        } else {
                            mac::text_secondary()
                        },
                    ))
                    .child(figure(
                        format_rate(interface.sent_rate),
                        if active {
                            teal.into()
                        } else {
                            mac::text_secondary()
                        },
                    ))
                    .into_any_element()
            })
            .collect();

        div().flex_1().min_h(px(0.0)).px_4().pb_4().child(
            div()
                .id("net-iface-table")
                .size_full()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .border_1()
                .border_color(mac::separator())
                .rounded(px(mac::radius_control()))
                .bg(mac::window())
                .child(header)
                .children(rows),
        )
    }
}
