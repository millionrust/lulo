//! System Settings bounded battery-history card projection.

use super::*;

pub(in crate::controller) fn battery_history_card(
    points: &[rmac_power::BatteryHistoryPoint],
) -> Div {
    let samples = sample_battery_history(points, 48);
    let minimum = points
        .iter()
        .map(|point| point.percentage)
        .min()
        .unwrap_or_default();
    let maximum = points
        .iter()
        .map(|point| point.percentage)
        .max()
        .unwrap_or_default();
    let latest = points
        .last()
        .map(|point| point.percentage)
        .unwrap_or_default();
    let bars = samples.into_iter().map(|point| {
        let color = if matches!(
            point.state,
            rmac_power::BatteryState::Charging | rmac_power::BatteryState::PendingCharge
        ) {
            hsl(0x34c759)
        } else {
            hsl(0x34c759).opacity(0.75)
        };
        div()
            .flex_1()
            .min_w(px(2.0))
            .h(px(4.0 + f32::from(point.percentage) * 0.86))
            .rounded(px(1.0))
            .bg(color)
    });
    // macOS 26: the history sits in its own group under a bold "Battery
    // Level" title, green bars on a 90 pt plot.
    div()
        .v_flex()
        .mb(px(style::GROUP_GAP))
        .gap(px(8.0))
        .p(px(style::ROW_PADDING))
        .rounded(px(style::GROUP_RADIUS))
        .bg(card_bg())
        .child(
            div()
                .v_flex()
                .gap(px(2.0))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .line_height(px(16.0))
                        .font_weight(rmac_ui::mac::BOLD)
                        .text_color(label())
                        .child("Battery Level"),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(11.0))
                        .line_height(px(14.0))
                        .text_color(secondary())
                        .child(format!(
                            "Last 24 hours · {minimum}% minimum · {maximum}% maximum · {latest}% latest"
                        )),
                ),
        )
        .child(
            div()
                .h(px(90.0))
                .flex()
                .items_end()
                .gap(px(2.0))
                .children(bars),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .text_size(rmac_ui::text_px(10.5))
                .text_color(rmac_ui::mac::text_tertiary())
                .child("24 hours ago")
                .child("Now"),
        )
}
