//! Window ▸ CPU Usage / CPU History / GPU History (MON-10/MON-14,
//! MON-MENU-033/034/035): small floating windows that mirror the real
//! numbers the main window already samples, so opening one adds a view,
//! never a second source of truth. Each window observes the live
//! `MonitorView` entity and redraws on every refresh tick; closing it is
//! an ordinary window close, with no extra state kept here.

use gpui::{
    div, px, App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Styled as _, Window, WindowHandle,
};
use rmac_ui::{mac, Root, StyledExt as _};

use crate::gpu_stats::GpuReading;
use crate::view::MonitorView;

const WIDTH: f32 = 280.0;
const HEIGHT: f32 = 200.0;
const GRAPH_HEIGHT: f32 = 120.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MetricWindowKind {
    CpuUsage,
    CpuHistory,
    GpuHistory,
}

impl MetricWindowKind {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::CpuUsage => "CPU Usage",
            Self::CpuHistory => "CPU History",
            Self::GpuHistory => "GPU History",
        }
    }

    /// Stable id used only by `quit_and_keep_windows.rs`'s persisted list;
    /// never shown to the user.
    pub(crate) fn storage_id(self) -> &'static str {
        match self {
            Self::CpuUsage => "cpu-usage",
            Self::CpuHistory => "cpu-history",
            Self::GpuHistory => "gpu-history",
        }
    }

    pub(crate) fn from_storage_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.storage_id() == id)
    }

    pub(crate) const ALL: [Self; 3] = [Self::CpuUsage, Self::CpuHistory, Self::GpuHistory];

    fn index(self) -> usize {
        match self {
            Self::CpuUsage => 0,
            Self::CpuHistory => 1,
            Self::GpuHistory => 2,
        }
    }
}

// One slot per `MetricWindowKind`, the same open-at-most-one-window
// pattern `calendar/src/settings_window.rs` and `calculator/src/
// maths_notes.rs` already use for their own singleton windows. A stale
// handle (the user already closed the window) is detected the same way
// those do: `WindowHandle::update` fails once the window is gone, with no
// separate close hook needed.
thread_local! {
    static OPEN: std::cell::RefCell<[Option<WindowHandle<Root>>; 3]> =
        const { std::cell::RefCell::new([None, None, None]) };
}

/// Open (or just focus, if one is already open) a floating metric window.
/// Lulo keeps at most one of each kind per the main window, matching the
/// Mac's own CPU/GPU windows.
pub(crate) fn show(kind: MetricWindowKind, monitor: Entity<MonitorView>, cx: &mut App) {
    let existing = OPEN.with(|open| open.borrow()[kind.index()]);
    if let Some(handle) = existing {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    // Not `window_options_for_app`: this floating window would then inherit
    // whatever size the main Activity Monitor window last saved under the
    // same app_id (the UIA-06/UIA-09 window-geometry-key bug).
    let options =
        rmac_ui::window_options_for_panel(rmac_ui::app_id::SYSTEM_MONITOR, WIDTH, HEIGHT, cx);
    match cx.open_window(options, move |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| MetricWindowView::new(kind, monitor, window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        Ok(handle) => OPEN.with(|open| open.borrow_mut()[kind.index()] = Some(handle)),
        Err(error) => eprintln!(
            "rmac-system-monitor: could not open {}: {error}",
            kind.title()
        ),
    }
}

/// Every kind whose window is genuinely still open right now — used only
/// by Quit and Keep Windows to decide what to persist. A read-only probe
/// (never `activate_window`): this must not steal focus just to check
/// liveness.
pub(crate) fn open_kinds(cx: &App) -> Vec<MetricWindowKind> {
    MetricWindowKind::ALL
        .into_iter()
        .filter(|kind| {
            OPEN.with(|open| open.borrow()[kind.index()])
                .is_some_and(|handle| handle.read_with(cx, |_, _| ()).is_ok())
        })
        .collect()
}

struct MetricWindowView {
    kind: MetricWindowKind,
    monitor: Entity<MonitorView>,
    focus: FocusHandle,
}

impl MetricWindowView {
    fn new(
        kind: MetricWindowKind,
        monitor: Entity<MonitorView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        window.set_window_title(kind.title());
        cx.observe(&monitor, |_, _, cx| cx.notify()).detach();
        Self {
            kind,
            monitor,
            focus: cx.focus_handle(),
        }
    }
}

/// A simple bottom-aligned bar graph, self-contained rather than reusing
/// `view/render/metrics_panes.rs`'s private helpers (that module is a
/// descendant of `view`, not reachable from here — see the accessor
/// methods this calls on `MonitorView` in `view.rs`).
fn bar_graph(samples: &[f32], max: f32, color: gpui::Hsla) -> impl IntoElement {
    let max = max.max(1.0);
    div()
        .w_full()
        .h(px(GRAPH_HEIGHT))
        .flex()
        .items_end()
        .gap(px(1.0))
        .children(samples.iter().map(move |&value| {
            let height = (value / max).clamp(0.0, 1.0) * GRAPH_HEIGHT;
            div().flex_1().h(px(height.max(1.0))).bg(color)
        }))
}

fn stat_row(label: &str, value: String) -> impl IntoElement {
    div()
        .h_flex()
        .items_center()
        .justify_between()
        .text_size(rmac_ui::text_px(12.0))
        .child(
            div()
                .text_color(mac::text_secondary())
                .child(label.to_string()),
        )
        .child(div().text_color(mac::text()).child(value))
}

impl Render for MetricWindowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let monitor = self.monitor.read(cx);
        let body: gpui::AnyElement = match self.kind {
            MetricWindowKind::CpuUsage => {
                let (user, system) = match monitor.cpu_split() {
                    Some((user, system, _idle)) => (format!("{user:.1}%"), format!("{system:.1}%")),
                    None => ("—".to_string(), "—".to_string()),
                };
                div()
                    .v_flex()
                    .gap_2()
                    .child(stat_row("User", user))
                    .child(stat_row("System", system))
                    .into_any_element()
            }
            MetricWindowKind::CpuHistory => {
                let (user_history, system_history) = monitor.cpu_history();
                let combined: Vec<f32> = user_history
                    .iter()
                    .zip(system_history.iter())
                    .map(|(u, s)| u + s)
                    .collect();
                div()
                    .v_flex()
                    .gap_2()
                    .child(bar_graph(&combined, 100.0, mac::accent()))
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .text_color(mac::text_secondary())
                            .child("Combined CPU load (User + System), last 60 samples"),
                    )
                    .into_any_element()
            }
            MetricWindowKind::GpuHistory => match monitor.gpu_reading() {
                GpuReading::Unavailable => div()
                    .v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(mac::text_secondary())
                            .child(
                                "No supported GPU usage source was found on this system \
                                 (amdgpu's gpu_busy_percent is not present, and Intel/i915 \
                                 per-process GPU accounting is not read)."
                                    .to_string(),
                            ),
                    )
                    .into_any_element(),
                GpuReading::Percent(_) => {
                    let history = monitor.gpu_history();
                    div()
                        .v_flex()
                        .gap_2()
                        .child(bar_graph(history, 100.0, mac::accent()))
                        .child(stat_row(
                            "GPU Busy",
                            history
                                .last()
                                .map(|v| format!("{v:.0}%"))
                                .unwrap_or_else(|| "—".to_string()),
                        ))
                        .into_any_element()
                }
            },
        };
        div()
            .track_focus(&self.focus)
            .size_full()
            .v_flex()
            .bg(mac::window())
            .text_color(mac::text())
            .child(rmac_ui::title_bar_content(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.0))
                    .font_weight(mac::SEMIBOLD)
                    .child(self.kind.title()),
            ))
            .child(div().flex_1().p_3().child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_ids_round_trip_for_every_kind() {
        for kind in MetricWindowKind::ALL {
            assert_eq!(
                MetricWindowKind::from_storage_id(kind.storage_id()),
                Some(kind)
            );
        }
        assert_eq!(MetricWindowKind::from_storage_id("unknown"), None);
    }

    #[test]
    fn every_kind_has_a_distinct_title_and_storage_id() {
        let titles: Vec<_> = MetricWindowKind::ALL.iter().map(|k| k.title()).collect();
        let ids: Vec<_> = MetricWindowKind::ALL
            .iter()
            .map(|k| k.storage_id())
            .collect();
        for title in &titles {
            assert_eq!(titles.iter().filter(|t| *t == title).count(), 1);
        }
        for id in &ids {
            assert_eq!(ids.iter().filter(|i| *i == id).count(), 1);
        }
    }
}
