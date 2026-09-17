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
            accent()
        };
        div()
            .flex_1()
            .min_w(px(2.0))
            .h(px(4.0 + f32::from(point.percentage) * 0.72))
            .rounded(px(mac::radius_menu_item()))
            .bg(color)
    });
    div()
        .v_flex()
        .mb_3()
        .gap_2()
        .p_3()
        .rounded(px(mac::radius_menu()))
        .bg(card_bg())
        .border_1()
        .border_color(sep())
        .child(
            div()
                .text_size(rmac_ui::text_px(12.0))
                .text_color(secondary())
                .child(format!(
                    "Last 24 hours · {minimum}% minimum · {maximum}% maximum · {latest}% latest"
                )),
        )
        .child(
            div()
                .h(px(80.0))
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
