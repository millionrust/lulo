//! System Settings Battery charts: the macOS 26 Battery Level and Screen
//! On Usage bar charts, with the value axis on the right and time ticks
//! underneath.

use super::*;

/// One bar: its height as a fraction of the plot, its colour, and an
/// optional marker under it (the charging bolt).
pub(in crate::controller) struct ChartBar {
    pub(in crate::controller) fraction: f32,
    pub(in crate::controller) color: Hsla,
    pub(in crate::controller) marked: bool,
}

const PLOT_HEIGHT: f32 = 90.0;
const AXIS_WIDTH: f32 = 40.0;
const TICK_HEIGHT: f32 = 14.0;
const MARKER_HEIGHT: f32 = 4.0;

/// A titled bar chart in the Mac's layout: bold 13 pt title, the plot with
/// faint guides, `axis` labels (top, middle, bottom) on the right, and
/// `ticks` (bar index, label) under it. `summary` is what a screen reader
/// hears for the whole chart.
pub(in crate::controller) fn bar_chart(
    id: &'static str,
    title: &'static str,
    summary: SharedString,
    bars: Vec<ChartBar>,
    axis: [&'static str; 3],
    ticks: Vec<(usize, SharedString)>,
) -> AnyElement {
    let count = bars.len().max(1);
    let has_markers = bars.iter().any(|bar| bar.marked);
    let plot = div()
        .relative()
        .flex_1()
        .h(px(PLOT_HEIGHT))
        .border_b_1()
        .border_color(sep())
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top_0()
                .h(px(1.0))
                .bg(sep()),
        )
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(PLOT_HEIGHT / 2.0))
                .h(px(1.0))
                .bg(sep()),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_end()
                .gap(px(1.0))
                .children(bars.iter().map(|bar| {
                    div()
                        .flex_1()
                        .h(px((bar.fraction.clamp(0.0, 1.0) * PLOT_HEIGHT).max(0.0)))
                        .bg(bar.color)
                })),
        );
    let markers = has_markers.then(|| {
        div()
            .flex()
            .gap(px(1.0))
            .h(px(MARKER_HEIGHT))
            .mt(px(2.0))
            .mr(px(AXIS_WIDTH))
            .children(bars.iter().map(|bar| {
                div()
                    .flex_1()
                    .h_full()
                    .when(bar.marked, |marker| marker.bg(rmac_ui::mac::system_green()))
            }))
    });
    let axis_labels = div()
        .w(px(AXIS_WIDTH))
        .h(px(PLOT_HEIGHT))
        .pl(px(6.0))
        .flex()
        .flex_col()
        .justify_between()
        .text_size(rmac_ui::text_px(10.5))
        .text_color(secondary())
        .children(axis);
    let tick_row = div()
        .relative()
        .h(px(TICK_HEIGHT))
        .mt(px(4.0))
        .mr(px(AXIS_WIDTH))
        .text_size(rmac_ui::text_px(10.5))
        .text_color(secondary())
        .children(ticks.into_iter().map(|(index, label)| {
            div()
                .absolute()
                .left(gpui::relative(index as f32 / count as f32))
                .child(label)
        }));
    div()
        .id(id)
        .role(gpui::Role::Group)
        .aria_label(summary)
        .v_flex()
        .gap(px(6.0))
        .child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .line_height(px(16.0))
                .font_weight(rmac_ui::mac::BOLD)
                .text_color(label())
                .child(title),
        )
        .child(div().flex().child(plot).child(axis_labels))
        .when_some(markers, |chart, markers| chart.child(markers))
        .child(tick_row)
        .into_any_element()
}
