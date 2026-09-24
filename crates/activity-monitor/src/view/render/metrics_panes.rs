//! Activity Monitor's bottom summary panel and the system-wide network pane.
//!
//! Measured on macOS 26.2 (design-lab/apps.html): a 117 pt panel under a
//! 1 pt line holds one bordered box, centred, 13 pt below the line and 89 pt
//! tall. Statistic cells are 198 wide with 13 pt "Label:  value" rows on a
//! 20 pt pitch separated by hairlines inset 9; graph cells are 184 wide with
//! a 10 pt bold capitalised title over a hairline inset 12 and the graph
//! below. CPU LOAD stacks User (#6DD0F9 over #3C5B6F) on System (#EB534D
//! over #715865); I/O graphs mirror reads/received above a centre line and
//! writes/sent below it.

mod network;

use super::*;
use crate::metrics::History;
use gpui::AnyElement;

const PANEL_HEIGHT: f32 = 117.0;
const BOX_TOP: f32 = 13.0;
const BOX_HEIGHT: f32 = 89.0;
const STATS_CELL_WIDTH: f32 = 198.0;
const GRAPH_CELL_WIDTH: f32 = 184.0;
const STAT_ROW_HEIGHT: f32 = 20.0;
const STAT_TOP_PADDING: f32 = 3.0;
const STAT_INSET: f32 = 9.0;
const GRAPH_TITLE_HEIGHT: f32 = 22.0;
const GRAPH_TITLE_INSET: f32 = 12.0;
const GRAPH_INSET: f32 = 3.0;

fn dark() -> bool {
    mac::window().l < 0.5
}

/// The panel's rules: #A6A6A6 in dark mode (measured); light mode is not
/// measured (S).
fn rule() -> gpui::Hsla {
    if dark() {
        gpui::rgb(0xa6a6a6).into()
    } else {
        gpui::rgb(0xb8b8b8).into()
    }
}

/// The box fill, a step above the window: #272934 on #1E202C.
fn box_fill() -> gpui::Hsla {
    if dark() {
        gpui::hsla(0.0, 0.0, 1.0, 0.04)
    } else {
        gpui::hsla(0.0, 0.0, 0.0, 0.03)
    }
}

#[derive(Clone, Copy)]
struct Series {
    line: gpui::Hsla,
    fill: gpui::Hsla,
}

fn blue() -> Series {
    Series {
        line: gpui::rgb(0x6dd0f9).into(),
        fill: gpui::rgb(0x3c5b6f).into(),
    }
}

fn red() -> Series {
    Series {
        line: gpui::rgb(0xeb534d).into(),
        fill: gpui::rgb(0x715865).into(),
    }
}

fn green() -> Series {
    Series {
        line: gpui::rgb(0x52a050).into(),
        fill: gpui::rgb(0x3a6837).into(),
    }
}

/// How a graph cell draws its samples.
enum Graph<'a> {
    /// One series filled up from the bottom, scaled to `max`.
    Area {
        samples: &'a [f32],
        max: f32,
        series: Series,
    },
    /// Two percentages stacked from the bottom (System under User).
    Stacked {
        lower: &'a [f32],
        upper: &'a [f32],
        lower_series: Series,
        upper_series: Series,
    },
    /// Two rates mirrored about a centre line, scaled together.
    Mirrored { above: &'a [f32], below: &'a [f32] },
}

/// `samples` aligned to the newest column on the right, padded with `None`
/// on the left until the history fills.
fn aligned(samples: &[f32]) -> impl Iterator<Item = Option<f32>> + '_ {
    let start = samples.len().saturating_sub(History::CAP);
    let visible = &samples[start..];
    std::iter::repeat(None)
        .take(History::CAP - visible.len())
        .chain(visible.iter().copied().map(Some))
}

fn bar(height: f32, series: Series, top_line: bool) -> gpui::Div {
    let height = height.max(0.0);
    div()
        .w_full()
        .h(px(height))
        .flex_none()
        .bg(series.fill)
        .when(height >= 1.0, |bar| {
            if top_line {
                bar.border_t_1().border_color(series.line)
            } else {
                bar.border_b_1().border_color(series.line)
            }
        })
}

fn format_count(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

impl MonitorView {
    fn stats_cell(&self, rows: Vec<(&'static str, String, Option<gpui::Hsla>)>) -> AnyElement {
        let count = rows.len();
        div()
            .w(px(STATS_CELL_WIDTH))
            .h_full()
            .flex_none()
            .pt(px(STAT_TOP_PADDING))
            .px(px(STAT_INSET))
            .v_flex()
            .children(
                rows.into_iter()
                    .enumerate()
                    .map(|(index, (label, value, color))| {
                        div()
                            .h(px(STAT_ROW_HEIGHT))
                            .flex_none()
                            .h_flex()
                            .items_center()
                            .justify_between()
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(mac::text())
                            .when(index + 1 < count, |row| {
                                row.border_b_1().border_color(rule())
                            })
                            .child(label)
                            .child(
                                div()
                                    .text_color(color.unwrap_or_else(mac::text))
                                    .child(value),
                            )
                    }),
            )
            .into_any_element()
    }

    fn graph_cell(&self, title: &'static str, graph: Graph<'_>) -> AnyElement {
        let height = BOX_HEIGHT - 2.0 - GRAPH_TITLE_HEIGHT - GRAPH_INSET;
        let columns: Vec<AnyElement> = match graph {
            Graph::Area {
                samples,
                max,
                series,
            } => {
                let max = max.max(f32::EPSILON);
                aligned(samples)
                    .map(|sample| {
                        div()
                            .flex_1()
                            .h_full()
                            .flex()
                            .flex_col()
                            .justify_end()
                            .when_some(sample, |column, value| {
                                column.child(bar(
                                    (value / max).clamp(0.0, 1.0) * height,
                                    series,
                                    true,
                                ))
                            })
                            .into_any_element()
                    })
                    .collect()
            }
            Graph::Stacked {
                lower,
                upper,
                lower_series,
                upper_series,
            } => aligned(lower)
                .zip(aligned(upper))
                .map(|(low, high)| {
                    div()
                        .flex_1()
                        .h_full()
                        .flex()
                        .flex_col()
                        .justify_end()
                        .when_some(high, |column, value| {
                            column.child(bar(
                                (value / 100.0).clamp(0.0, 1.0) * height,
                                upper_series,
                                true,
                            ))
                        })
                        .when_some(low, |column, value| {
                            column.child(bar(
                                (value / 100.0).clamp(0.0, 1.0) * height,
                                lower_series,
                                true,
                            ))
                        })
                        .into_any_element()
                })
                .collect(),
            Graph::Mirrored { above, below } => {
                let max = above
                    .iter()
                    .chain(below)
                    .copied()
                    .fold(f32::EPSILON, f32::max);
                let half = height / 2.0;
                aligned(above)
                    .zip(aligned(below))
                    .map(|(up, down)| {
                        div()
                            .flex_1()
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(div().h(px(half)).flex().flex_col().justify_end().when_some(
                                up,
                                |half_column, value| {
                                    half_column.child(bar(
                                        (value / max).clamp(0.0, 1.0) * half,
                                        blue(),
                                        true,
                                    ))
                                },
                            ))
                            .child(div().h(px(half)).flex().flex_col().when_some(
                                down,
                                |half_column, value| {
                                    half_column.child(bar(
                                        (value / max).clamp(0.0, 1.0) * half,
                                        red(),
                                        false,
                                    ))
                                },
                            ))
                            .into_any_element()
                    })
                    .collect()
            }
        };
        div()
            .w(px(GRAPH_CELL_WIDTH))
            .h_full()
            .flex_none()
            .v_flex()
            .child(
                div().px(px(GRAPH_TITLE_INSET)).child(
                    div()
                        .h(px(GRAPH_TITLE_HEIGHT))
                        .flex()
                        .items_center()
                        .justify_center()
                        .border_b_1()
                        .border_color(rule())
                        .text_size(rmac_ui::text_px(10.0))
                        .font_weight(mac::BOLD)
                        .text_color(mac::text_secondary())
                        .child(title),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .px(px(GRAPH_INSET))
                    .pb(px(GRAPH_INSET))
                    .flex()
                    .items_end()
                    .child(div().w_full().h(px(height)).flex().children(columns)),
            )
            .into_any_element()
    }

    fn cpu_cells(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let history = &self.sampler.history;
        let aggregates = &self.sampler.aggregates;
        // CPU use is a difference between two readings, so the first one
        // shows a dash rather than an idle machine that was never measured.
        let (system, user, idle) = match self.sampler.cpu_split {
            Some((user, system, idle)) => (
                format!("{system:.2}%"),
                format!("{user:.2}%"),
                format!("{idle:.2}%"),
            ),
            None => ("—".to_string(), "—".to_string(), "—".to_string()),
        };
        let mut counts = Vec::new();
        if let Some(threads) = aggregates.threads {
            counts.push(("Threads:", format_count(threads), None));
        }
        counts.push((
            "Processes:",
            format_count(self.table.read(cx).delegate().all_rows.len() as u64),
            None,
        ));
        vec![
            self.stats_cell(vec![
                ("System:", system, Some(red().line)),
                ("User:", user, Some(blue().line)),
                ("Idle:", idle, None),
            ]),
            self.graph_cell(
                "CPU LOAD",
                Graph::Stacked {
                    lower: &history.cpu_system,
                    upper: &history.cpu_user,
                    lower_series: red(),
                    upper_series: blue(),
                },
            ),
            self.stats_cell(counts),
        ]
    }

    fn memory_cells(&self) -> Vec<AnyElement> {
        let aggregates = &self.sampler.aggregates;
        let used = if aggregates.mem_total > 0 {
            aggregates.mem_used as f32 / aggregates.mem_total as f32
        } else {
            0.0
        };
        // Green while under 60 % in use, then the Mac's yellow and red bands.
        let series = if used < 0.60 {
            green()
        } else if used < 0.80 {
            Series {
                line: mac::system_yellow(),
                fill: mac::system_yellow().opacity(0.45),
            }
        } else {
            red()
        };
        let mut rows = vec![
            ("Physical Memory:", format_mem(aggregates.mem_total), None),
            ("Memory Used:", format_mem(aggregates.mem_used), None),
        ];
        if let Some(cached) = aggregates.mem_cached {
            rows.push(("Cached Files:", format_mem(cached), None));
        }
        rows.push(("Swap Used:", format_mem(aggregates.swap_used), None));
        vec![
            // rmac graphs the share of memory in use; the Mac's pressure
            // metric is private, so the title says what is drawn.
            self.graph_cell(
                "MEMORY USED",
                Graph::Area {
                    samples: &self.sampler.history.mem,
                    max: 100.0,
                    series,
                },
            ),
            self.stats_cell(rows),
        ]
    }

    fn energy_cells(&self) -> Vec<AnyElement> {
        let samples = &self.sampler.history.energy;
        let max = samples.iter().copied().fold(1.0f32, f32::max);
        vec![self.graph_cell(
            "ENERGY IMPACT",
            Graph::Area {
                samples,
                max,
                series: blue(),
            },
        )]
    }

    fn disk_cells(&self) -> Vec<AnyElement> {
        let aggregates = &self.sampler.aggregates;
        vec![
            self.graph_cell(
                "DATA",
                Graph::Mirrored {
                    above: &self.sampler.history.disk_read,
                    below: &self.sampler.history.disk_write,
                },
            ),
            self.stats_cell(vec![
                (
                    "Data read/sec:",
                    format_bytes(aggregates.disk_read_rate as u64),
                    Some(blue().line),
                ),
                (
                    "Data written/sec:",
                    format_bytes(aggregates.disk_write_rate as u64),
                    Some(red().line),
                ),
            ]),
        ]
    }

    fn network_cells(&self) -> Vec<AnyElement> {
        let aggregates = &self.sampler.aggregates;
        vec![
            self.graph_cell(
                "DATA",
                Graph::Mirrored {
                    above: &self.sampler.history.net_recv,
                    below: &self.sampler.history.net_sent,
                },
            ),
            self.stats_cell(vec![
                (
                    "Data received:",
                    format_bytes(aggregates.net_total_recv),
                    None,
                ),
                ("Data sent:", format_bytes(aggregates.net_total_sent), None),
                (
                    "Data received/sec:",
                    format_bytes(aggregates.net_recv_rate as u64),
                    Some(blue().line),
                ),
                (
                    "Data sent/sec:",
                    format_bytes(aggregates.net_sent_rate as u64),
                    Some(red().line),
                ),
            ]),
        ]
    }

    pub(super) fn render_bottom_panel(&self, cx: &Context<Self>) -> impl IntoElement {
        let cells = match self.tab {
            Tab::Cpu => self.cpu_cells(cx),
            Tab::Memory => self.memory_cells(),
            Tab::Energy => self.energy_cells(),
            Tab::Disk => self.disk_cells(),
            Tab::Network => self.network_cells(),
        };
        let cell_count = cells.len();
        div()
            .h(px(PANEL_HEIGHT))
            .flex_none()
            .border_t_1()
            .border_color(rule())
            .pt(px(BOX_TOP - 1.0))
            .flex()
            .justify_center()
            .child(
                div()
                    .h(px(BOX_HEIGHT))
                    .flex()
                    .border_1()
                    .border_color(rule())
                    .bg(box_fill())
                    .children(cells.into_iter().enumerate().map(|(index, cell)| {
                        div()
                            .h_full()
                            .when(index > 0 && index < cell_count, |divided| {
                                divided.border_l_1().border_color(rule())
                            })
                            .child(cell)
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::format_count;

    #[test]
    fn counts_group_thousands_like_activity_monitor() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(763), "763");
        assert_eq!(format_count(3159), "3,159");
        assert_eq!(format_count(1_234_567), "1,234,567");
    }
}
