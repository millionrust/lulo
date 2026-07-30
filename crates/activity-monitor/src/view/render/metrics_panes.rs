//! System Monitor summary, CPU, memory, and network presentation.

use super::*;

impl MonitorView {
    pub(super) fn stat_card(
        &self,
        label: &str,
        value: String,
        accent: gpui::Hsla,
    ) -> impl IntoElement {
        div()
            .v_flex()
            .gap_1()
            .px_4()
            .py_3()
            .min_w(px(150.0))
            .rounded(px(10.0))
            .bg(mac::chrome())
            .border_1()
            .border_color(mac::separator())
            .child(
                div()
                    .text_size(rmac_ui::text_px(11.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text_tertiary())
                    .child(label.to_uppercase()),
            )
            .child(
                div()
                    .text_size(rmac_ui::text_px(24.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(accent)
                    .child(value),
            )
    }

    pub(super) fn sparkline(&self, samples: &[f32], accent: gpui::Hsla) -> impl IntoElement {
        let max = samples.iter().cloned().fold(1.0f32, f32::max);
        let bars: Vec<_> = samples
            .iter()
            .map(|&value| {
                let fraction = (value / max).clamp(0.02, 1.0);
                div()
                    .flex_1()
                    .h(px(40.0 * fraction))
                    .min_w(px(2.0))
                    .rounded(px(1.0))
                    .bg(accent)
            })
            .collect();
        div()
            .h(px(44.0))
            .w_full()
            .flex()
            .items_end()
            .gap(px(1.0))
            .px_1()
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(px(1.0))
                    .size_full()
                    .children(bars),
            )
    }

    pub(super) fn render_core_bars(&self) -> impl IntoElement {
        let blue = gpui::rgb(0x007aff);
        let cores = self.sampler.aggregates.per_core.clone();
        div()
            .v_flex()
            .gap_2()
            .pt_3()
            .child(
                div()
                    .text_size(rmac_ui::text_px(11.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text_tertiary())
                    .child("CPU CORES"),
            )
            .child(div().flex().flex_wrap().gap_x_4().gap_y_2().children(
                cores.into_iter().enumerate().map(|(index, usage)| {
                    let fraction = (usage / 100.0).clamp(0.0, 1.0);
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .w(px(160.0))
                        .child(
                            div()
                                .w(px(48.0))
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(mac::text_secondary())
                                .child(format!("Core {}", index + 1)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .h(px(6.0))
                                .rounded(px(3.0))
                                .bg(mac::chrome())
                                .child(
                                    div()
                                        .h_full()
                                        .w(gpui::relative(fraction))
                                        .rounded(px(3.0))
                                        .bg(blue),
                                ),
                        )
                        .child(
                            div()
                                .w(px(34.0))
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(mac::text())
                                .child(format!("{usage:.0}%")),
                        )
                }),
            ))
    }

    /// Network is system-wide because the current authority has no reliable
    /// per-process network accounting.
    pub(super) fn render_network_pane(&self) -> impl IntoElement {
        let teal = gpui::rgb(0x32ade6);
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
                .rounded(px(8.0))
                .bg(mac::window())
                .child(header)
                .children(rows),
        )
    }

    pub(super) fn render_summary(&self, cx: &Context<Self>) -> impl IntoElement {
        let blue = gpui::rgb(0x007aff).into();
        let green = gpui::rgb(0x28b463).into();
        let orange = gpui::rgb(0xff9500).into();
        let purple = gpui::rgb(0xaf52de).into();
        let teal = gpui::rgb(0x32ade6).into();

        let (cards, samples, accent): (Vec<gpui::AnyElement>, &[f32], gpui::Hsla) = match self.tab {
            Tab::Cpu => {
                let red = gpui::rgb(0xff3b30).into();
                let mut cards = vec![
                    self.stat_card(
                        "CPU Load",
                        format!("{:.1}%", self.sampler.aggregates.cpu_total),
                        blue,
                    )
                    .into_any_element(),
                    self.stat_card(
                        "Cores",
                        self.sampler.aggregates.per_core.len().to_string(),
                        mac::text(),
                    )
                    .into_any_element(),
                    self.stat_card(
                        "Processes",
                        self.table.read(cx).delegate().all_rows.len().to_string(),
                        mac::text(),
                    )
                    .into_any_element(),
                ];
                if let Some((user, system, idle)) = self.sampler.cpu_split {
                    cards.push(
                        self.stat_card("System", format!("{system:.1}%"), red)
                            .into_any_element(),
                    );
                    cards.push(
                        self.stat_card("User", format!("{user:.1}%"), blue)
                            .into_any_element(),
                    );
                    cards.push(
                        self.stat_card("Idle", format!("{idle:.1}%"), mac::text_secondary())
                            .into_any_element(),
                    );
                }
                (cards, self.sampler.history.cpu.as_slice(), blue)
            }
            Tab::Memory => {
                let cards = vec![
                    self.stat_card(
                        "Memory Used",
                        format!(
                            "{} / {}",
                            format_mem(self.sampler.aggregates.mem_used),
                            format_mem(self.sampler.aggregates.mem_total)
                        ),
                        green,
                    )
                    .into_any_element(),
                    self.stat_card(
                        "Available",
                        format_mem(self.sampler.aggregates.mem_available),
                        mac::text(),
                    )
                    .into_any_element(),
                    self.stat_card(
                        "Swap",
                        format!(
                            "{} / {}",
                            format_mem(self.sampler.aggregates.swap_used),
                            format_mem(self.sampler.aggregates.swap_total)
                        ),
                        mac::text(),
                    )
                    .into_any_element(),
                ];
                (cards, self.sampler.history.mem.as_slice(), green)
            }
            Tab::Energy => {
                let cards = vec![
                    self.stat_card(
                        "Energy Impact",
                        format!("{:.1}", self.sampler.aggregates.energy_total),
                        orange,
                    )
                    .into_any_element(),
                    self.stat_card(
                        "CPU Load",
                        format!("{:.1}%", self.sampler.aggregates.cpu_total),
                        mac::text(),
                    )
                    .into_any_element(),
                ];
                (cards, self.sampler.history.energy.as_slice(), orange)
            }
            Tab::Disk => {
                let cards = vec![
                    self.stat_card(
                        "Reads",
                        format_rate(self.sampler.aggregates.disk_read_rate),
                        purple,
                    )
                    .into_any_element(),
                    self.stat_card(
                        "Writes",
                        format_rate(self.sampler.aggregates.disk_write_rate),
                        purple,
                    )
                    .into_any_element(),
                ];
                (cards, self.sampler.history.disk.as_slice(), purple)
            }
            Tab::Network => {
                let cards = vec![
                    self.stat_card(
                        "Receiving",
                        format_rate(self.sampler.aggregates.net_recv_rate),
                        teal,
                    )
                    .into_any_element(),
                    self.stat_card(
                        "Sending",
                        format_rate(self.sampler.aggregates.net_sent_rate),
                        teal,
                    )
                    .into_any_element(),
                ];
                (cards, self.sampler.history.net.as_slice(), teal)
            }
        };

        let samples = samples.to_vec();
        div()
            .v_flex()
            .gap_3()
            .p_4()
            .border_b_1()
            .border_color(mac::separator())
            .child(div().h_flex().gap_3().children(cards))
            .child(self.sparkline(&samples, accent))
            .when(
                matches!(self.tab, Tab::Cpu) && !self.sampler.aggregates.per_core.is_empty(),
                |element| element.child(self.render_core_bars()),
            )
            .when(
                matches!(self.tab, Tab::Memory) && self.sampler.aggregates.mem_total > 0,
                |element| element.child(self.render_mem_pressure()),
            )
    }

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
