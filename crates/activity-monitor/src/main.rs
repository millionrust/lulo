//! rmac System Monitor — a fast, native, GPU-rendered process monitor.
//!
//! Phase 1 of the rmac desktop suite. Built on GPUI + gpui-component, with
//! `sysinfo` as the data layer. Runs on macOS today (Metal) and targets
//! Ubuntu/Wayland (Vulkan) later — the same binary, no webview, instant launch.

mod columns;
mod cpu_ticks;
mod metrics;
mod process_action;
mod process_signal;
mod process_table;
mod storage;

use std::time::Duration;

use gpui::{
    div, prelude::FluentBuilder as _, px, AppContext as _, Context, Entity,
    InteractiveElement as _, IntoElement, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, Window,
};
use gpui_component::StyledExt as _;
use rmac_ui::{mac, Button, InputState, SearchField, Table, TableEvent, TableState, Tabs};
use sysinfo::{Networks, Pid, ProcessRefreshKind, ProcessesToUpdate, Signal};

use columns::{default_visible as default_visible_cols, load as load_visible_cols};
use columns::{save as save_visible_cols, ColKey};
use metrics::{
    format_bytes, format_duration, format_mem, format_rate, Aggregates, History, NetIface, Tab,
    REFRESH_SECS,
};
use process_table::{resync_selection, ProcessTableDelegate};

gpui::actions!(
    activity_monitor,
    [
        QuitProcess,
        ForceQuitProcess,
        FocusSearch,
        CancelKill,
        ConfirmKill
    ]
);

fn process_signal_outcome(outcome: process_signal::SignalOutcome) -> process_action::Outcome {
    match outcome {
        process_signal::SignalOutcome::Delivered => process_action::Outcome::Delivered,
        process_signal::SignalOutcome::Missing => process_action::Outcome::Missing,
        process_signal::SignalOutcome::Unsupported => process_action::Outcome::Unsupported,
        process_signal::SignalOutcome::Rejected => process_action::Outcome::Rejected,
    }
}

/// Root view: a tabbed summary + the live process table.
struct MonitorView {
    table: Entity<TableState<ProcessTableDelegate>>,
    search: Entity<InputState>,
    focus: gpui::FocusHandle,
    networks: Networks,
    tab: Tab,
    agg: Aggregates,
    history: History,
    pending_kill: Option<process_action::Request>,
    process_action_feedback: Option<process_action::Feedback>,
    /// Whether the column chooser dropdown is open.
    cols_menu_open: bool,
    persistence_error: Option<SharedString>,
    /// PID whose detail inspector is open (double-click a row).
    inspect_pid: Option<u32>,
    /// Per-interface cumulative byte counters, snapshotted each refresh for the
    /// Network tab's interface table. (name, total received, total sent,
    /// received this interval, sent this interval).
    net_ifaces: Vec<NetIface>,
    /// Cumulative CPU ticks from the previous refresh, for the User/System/Idle
    /// delta. `cpu_split` is the latest (user%, system%, idle%) breakdown.
    prev_cpu_ticks: Option<[u64; 4]>,
    cpu_split: Option<(f32, f32, f32)>,
}

impl MonitorView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (visible, persistence_error) = match load_visible_cols() {
            Ok(visible) => (visible, None),
            Err(failure) => (
                default_visible_cols(),
                Some(SharedString::from(failure.to_string())),
            ),
        };
        let table = cx.new(|cx| TableState::new(ProcessTableDelegate::new(visible), window, cx));
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search name, PID or path"));

        // Re-filter live as the user types.
        cx.observe(&search, |this, _, cx| {
            this.apply_filter(cx);
        })
        .detach();

        // Keyboard navigation moves the highlighted row via `set_selected_row`,
        // which emits `SelectRow` but never touches `selected_pid`. Mirror the
        // click-selection path here so keyboard selection is the source of truth
        // for the target PID — otherwise a 2s background refresh could re-point
        // the highlighted row at a different process before a kill is requested.
        cx.subscribe(&table, |this, table, event: &TableEvent, cx| match event {
            TableEvent::SelectRow(row_ix) => {
                let row_ix = *row_ix;
                table.update(cx, |state, _| {
                    let pid = state.delegate().rows.get(row_ix).map(|r| r.pid);
                    state.delegate_mut().selected_pid = pid;
                });
            }
            TableEvent::DoubleClickedRow(row_ix) => {
                let pid = table.read(cx).delegate().rows.get(*row_ix).map(|r| r.pid);
                this.inspect_pid = pid;
                cx.notify();
            }
            _ => {}
        })
        .detach();

        let mut view = Self {
            table,
            search,
            focus: cx.focus_handle(),
            networks: Networks::new_with_refreshed_list(),
            tab: Tab::Cpu,
            agg: Aggregates::default(),
            history: History::default(),
            pending_kill: None,
            process_action_feedback: None,
            cols_menu_open: false,
            persistence_error,
            inspect_pid: None,
            net_ifaces: Vec::new(),
            prev_cpu_ticks: None,
            cpu_split: None,
        };
        view.refresh(cx);

        // Auto-refresh loop — every 2s, off the render path.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| loop {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let Some(this) = this.upgrade() else { break };
            let updated = cx.update_entity(&this, |view: &mut MonitorView, cx| {
                view.refresh(cx);
                cx.notify();
            });
            if updated.is_err() {
                break;
            }
        })
        .detach();

        view
    }

    fn apply_filter(&mut self, cx: &mut Context<Self>) {
        let q = self.search.read(cx).value().to_string();
        self.table.update(cx, |state, cx| {
            state.delegate_mut().set_filter(q);
            resync_selection(state, cx);
            state.refresh(cx);
        });
        cx.notify();
    }

    /// Refresh the table snapshot and recompute the summary aggregates.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        // Real host-wide CPU User/System/Idle split from the mach tick delta.
        if let Some(now) = cpu_ticks::read() {
            if let Some(prev) = self.prev_cpu_ticks {
                self.cpu_split = cpu_ticks::split(prev, now);
            }
            self.prev_cpu_ticks = Some(now);
        }

        self.networks.refresh(true);
        let (net_recv, net_sent) = self
            .networks
            .list()
            .values()
            .fold((0u64, 0u64), |(r, t), d| {
                (r + d.received(), t + d.transmitted())
            });

        // Per-interface snapshot for the Network tab table. `received()` /
        // `transmitted()` are the bytes since the previous refresh (the
        // interval delta); `total_*` are the cumulative counters.
        self.net_ifaces = self
            .networks
            .list()
            .iter()
            .map(|(name, d)| NetIface {
                name: name.clone(),
                total_recv: d.total_received(),
                total_sent: d.total_transmitted(),
                recv_rate: d.received() as f64 / REFRESH_SECS,
                sent_rate: d.transmitted() as f64 / REFRESH_SECS,
            })
            .collect();
        // Busiest interfaces first, then named order for stability.
        self.net_ifaces.sort_by(|a, b| {
            (b.total_recv + b.total_sent)
                .cmp(&(a.total_recv + a.total_sent))
                .then_with(|| a.name.cmp(&b.name))
        });

        let mut agg = Aggregates::default();
        self.table.update(cx, |state, cx| {
            let delegate = state.delegate_mut();
            delegate.refresh();
            let count = delegate.cpu_count as f32;

            agg.per_core = delegate
                .system
                .cpus()
                .iter()
                .map(|c| c.cpu_usage())
                .collect();
            // Sum of per-process CPU is ~total busy across all cores; normalise to 0-100%.
            agg.cpu_total =
                (delegate.all_rows.iter().map(|r| r.cpu).sum::<f32>() / count).min(100.0);
            agg.energy_total = delegate.all_rows.iter().map(|r| r.energy).sum::<f32>();
            agg.mem_used = delegate.system.used_memory();
            agg.mem_total = delegate.system.total_memory();
            agg.mem_available = delegate.system.available_memory();
            agg.swap_used = delegate.system.used_swap();
            agg.swap_total = delegate.system.total_swap();

            // Split is approximate; we only have the combined per-row figure here,
            // so re-derive read/write from the live processes.
            let (read, write) =
                delegate
                    .system
                    .processes()
                    .values()
                    .fold((0u64, 0u64), |(r, w), p| {
                        let du = p.disk_usage();
                        (r + du.read_bytes, w + du.written_bytes)
                    });
            agg.disk_read_rate = read as f64 / REFRESH_SECS;
            agg.disk_write_rate = write as f64 / REFRESH_SECS;

            // Snapshot just rebuilt/re-sorted — keep the highlight on the same PID.
            resync_selection(state, cx);
            state.refresh(cx);
        });

        agg.net_recv_rate = net_recv as f64 / REFRESH_SECS;
        agg.net_sent_rate = net_sent as f64 / REFRESH_SECS;

        // Append to history rings.
        History::push(&mut self.history.cpu, agg.cpu_total);
        let mem_pct = if agg.mem_total > 0 {
            (agg.mem_used as f32 / agg.mem_total as f32) * 100.0
        } else {
            0.0
        };
        History::push(&mut self.history.mem, mem_pct);
        History::push(&mut self.history.energy, agg.energy_total.min(100.0));
        History::push(
            &mut self.history.disk,
            ((agg.disk_read_rate + agg.disk_write_rate) / 1_048_576.0) as f32,
        );
        History::push(
            &mut self.history.net,
            ((agg.net_recv_rate + agg.net_sent_rate) / 1_048_576.0) as f32,
        );

        self.agg = agg;
    }

    fn select_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.cols_menu_open = false;
        self.table.update(cx, |state, cx| {
            let d = state.delegate_mut();
            let want = tab.default_sort_key();
            // Only adopt the tab's default sort if that column is visible.
            if d.visible.contains(&want) {
                d.sort_key = want;
            }
            d.sort_asc = false;
            d.apply_view();
            resync_selection(state, cx);
            state.refresh(cx);
        });
        cx.notify();
    }

    fn selected_proc(&self, cx: &Context<Self>) -> Option<process_action::ProcessIdentity> {
        let state = self.table.read(cx);
        let d = state.delegate();
        let pid = d.selected_pid?;
        d.rows
            .iter()
            .find(|r| r.pid == pid)
            .or_else(|| d.all_rows.iter().find(|r| r.pid == pid))
            .map(|row| process_action::ProcessIdentity {
                pid,
                start_time: row.start_time,
                name: row.name.to_string(),
            })
    }

    fn request_kill(&mut self, force: bool, cx: &mut Context<Self>) {
        // Capture whatever row is highlighted right now (covers keyboard nav,
        // which moves the table's selected row without touching `selected_pid`).
        self.table.update(cx, |state, _| {
            if let Some(pid) = state
                .selected_row()
                .and_then(|ix| state.delegate().rows.get(ix))
                .map(|r| r.pid)
            {
                state.delegate_mut().selected_pid = Some(pid);
            }
        });
        if let Some(process) = self.selected_proc(cx) {
            self.pending_kill = Some(process_action::Request {
                process,
                kind: if force {
                    process_action::ActionKind::ForceQuit
                } else {
                    process_action::ActionKind::Quit
                },
            });
            self.process_action_feedback = None;
            self.cols_menu_open = false;
            cx.notify();
        }
    }

    fn confirm_kill(&mut self, cx: &mut Context<Self>) {
        if let Some(request) = self.pending_kill.take() {
            let process_handle = process_signal::ProcessHandle::open(request.process.pid);
            let outcome =
                self.table.update(cx, |state, cx| {
                    let delegate = state.delegate_mut();
                    let pid = Pid::from_u32(request.process.pid);
                    delegate.system.refresh_processes_specifics(
                        ProcessesToUpdate::Some(&[pid]),
                        true,
                        ProcessRefreshKind::nothing(),
                    );
                    let observed = delegate.system.process(pid).map(|process| {
                        process_action::ProcessIdentity {
                            pid: process.pid().as_u32(),
                            start_time: process.start_time(),
                            name: process.name().to_string_lossy().into_owned(),
                        }
                    });
                    let outcome = match process_action::preflight(&request, observed.as_ref()) {
                        process_action::Preflight::Missing => process_action::Outcome::Missing,
                        process_action::Preflight::Replaced => process_action::Outcome::Replaced,
                        process_action::Preflight::Current => match process_handle {
                            Ok(handle) => process_signal_outcome(handle.send(
                                match request.kind {
                                    process_action::ActionKind::Quit => {
                                        process_signal::SignalKind::Terminate
                                    }
                                    process_action::ActionKind::ForceQuit => {
                                        process_signal::SignalKind::Kill
                                    }
                                },
                                || {
                                    let signal = match request.kind {
                                        process_action::ActionKind::Quit => Signal::Term,
                                        process_action::ActionKind::ForceQuit => Signal::Kill,
                                    };
                                    delegate
                                        .system
                                        .process(pid)
                                        .and_then(|process| process.kill_with(signal))
                                },
                            )),
                            Err(outcome) => process_signal_outcome(outcome),
                        },
                    };
                    if matches!(
                        outcome,
                        process_action::Outcome::Missing | process_action::Outcome::Replaced
                    ) {
                        delegate
                            .all_rows
                            .retain(|row| row.pid != request.process.pid);
                        delegate.apply_view();
                        delegate.selected_pid = None;
                        resync_selection(state, cx);
                        state.refresh(cx);
                    }
                    outcome
                });
            self.process_action_feedback = Some(process_action::feedback(&request, outcome));
            cx.notify();
        }
    }

    fn cancel_kill(&mut self, cx: &mut Context<Self>) {
        // Escape also dismisses the inspector and the column chooser.
        if self.pending_kill.take().is_some()
            || self.inspect_pid.take().is_some()
            || std::mem::take(&mut self.cols_menu_open)
        {
            cx.notify();
        }
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.focus(window, cx));
    }

    /// Toggle a column's visibility from the chooser, rebuilding the table layout.
    fn toggle_column(&mut self, key: ColKey, cx: &mut Context<Self>) {
        let visible = self.table.update(cx, |state, cx| {
            if state.delegate_mut().toggle_col(key) {
                state.delegate_mut().apply_view();
                resync_selection(state, cx);
                state.refresh(cx);
                Some(state.delegate().visible.clone())
            } else {
                None
            }
        });
        if let Some(visible) = visible {
            self.persistence_error = save_visible_cols(&visible)
                .err()
                .map(|failure| failure.to_string().into());
        }
        cx.notify();
    }

    /// The column-chooser dropdown: a checklist of every available column.
    fn render_columns_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let visible = self.table.read(cx).delegate().visible.clone();
        let top =
            96.0 + if self.persistence_error.is_some() {
                34.0
            } else {
                0.0
            } + if self.process_action_feedback.is_some() {
                52.0
            } else {
                0.0
            };
        div()
            .absolute()
            .top(px(top))
            .right(px(16.0))
            .w(px(210.0))
            .bg(mac::window())
            .rounded(px(8.0))
            .border_1()
            .border_color(mac::separator())
            .shadow_lg()
            .py_1()
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_size(rmac_ui::text_px(11.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text_tertiary())
                    .child("COLUMNS"),
            )
            .children(ColKey::ALL.into_iter().map(|key| {
                let on = visible.contains(&key);
                let disabled = key.required();
                div()
                    .id(SharedString::from(key.id()))
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .h(px(26.0))
                    .px_3()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(if disabled {
                        mac::text_tertiary()
                    } else {
                        mac::text()
                    })
                    .when(!disabled, |el: Stateful<gpui::Div>| {
                        el.hover(|h| h.bg(mac::chrome())).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.toggle_column(key, cx);
                            },
                        ))
                    })
                    .child(div().w(px(14.0)).text_color(mac::accent()).child(if on {
                        "✓"
                    } else {
                        ""
                    }))
                    .child(div().flex_1().child(key.title()))
            }))
    }

    fn stat_card(&self, label: &str, value: String, accent: gpui::Hsla) -> impl IntoElement {
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

    /// A compact bar sparkline of the most recent samples.
    fn sparkline(&self, samples: &[f32], accent: gpui::Hsla) -> impl IntoElement {
        let max = samples.iter().cloned().fold(1.0f32, f32::max);
        let bars: Vec<_> = samples
            .iter()
            .map(|&v| {
                let frac = (v / max).clamp(0.02, 1.0);
                div()
                    .flex_1()
                    .h(px(40.0 * frac))
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

    /// Per-core CPU usage bars for the CPU pane (real `sysinfo` per-core data).
    fn render_core_bars(&self) -> impl IntoElement {
        let blue = gpui::rgb(0x007aff);
        let cores = self.agg.per_core.clone();
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
                cores.into_iter().enumerate().map(|(i, usage)| {
                    let frac = (usage / 100.0).clamp(0.0, 1.0);
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
                                .child(format!("Core {}", i + 1)),
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
                                        .w(gpui::relative(frac))
                                        .rounded(px(3.0))
                                        .bg(blue),
                                ),
                        )
                        .child(
                            div()
                                .w(px(34.0))
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(mac::text())
                                .child(format!("{:.0}%", usage)),
                        )
                }),
            ))
    }

    /// The Network tab body: a real per-interface table (sysinfo `Networks`)
    /// since there is no reliable per-process network data on this platform.
    fn render_network_pane(&self) -> impl IntoElement {
        let teal = gpui::rgb(0x32ade6);
        // Column widths (interface label flexes, figures are fixed/right-aligned).
        let figure = |s: String, color: gpui::Hsla| {
            div()
                .w(px(110.0))
                .text_size(rmac_ui::text_px(12.0))
                .text_color(color)
                .text_right()
                .child(s)
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
            .children(["RCVD", "SENT", "↓ RATE", "↑ RATE"].into_iter().map(|h| {
                div()
                    .w(px(110.0))
                    .text_size(rmac_ui::text_px(11.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text_tertiary())
                    .text_right()
                    .child(h)
            }));

        let rows: Vec<gpui::AnyElement> = self
            .net_ifaces
            .iter()
            .enumerate()
            .map(|(i, iface)| {
                let active = iface.recv_rate + iface.sent_rate > 0.0;
                div()
                    .h_flex()
                    .items_center()
                    .px_3()
                    .py_1p5()
                    .when(i % 2 == 1, |el| el.bg(mac::hover()))
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
                                    .child(iface.name.clone()),
                            ),
                    )
                    .child(figure(format_bytes(iface.total_recv), mac::text()))
                    .child(figure(format_bytes(iface.total_sent), mac::text()))
                    .child(figure(
                        format_rate(iface.recv_rate),
                        if active {
                            teal.into()
                        } else {
                            mac::text_secondary()
                        },
                    ))
                    .child(figure(
                        format_rate(iface.sent_rate),
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

    fn render_summary(&self, cx: &Context<Self>) -> impl IntoElement {
        let blue = gpui::rgb(0x007aff).into();
        let green = gpui::rgb(0x28b463).into();
        let orange = gpui::rgb(0xff9500).into();
        let purple = gpui::rgb(0xaf52de).into();
        let teal = gpui::rgb(0x32ade6).into();

        let (cards, samples, accent): (Vec<gpui::AnyElement>, &[f32], gpui::Hsla) = match self.tab {
            Tab::Cpu => {
                let red = gpui::rgb(0xff3b30).into();
                let mut cards = vec![
                    self.stat_card("CPU Load", format!("{:.1}%", self.agg.cpu_total), blue)
                        .into_any_element(),
                    self.stat_card("Cores", self.agg.per_core.len().to_string(), mac::text())
                        .into_any_element(),
                    self.stat_card(
                        "Processes",
                        self.table.read(cx).delegate().all_rows.len().to_string(),
                        mac::text(),
                    )
                    .into_any_element(),
                ];
                // Real User/System/Idle split (mach host_statistics tick delta).
                if let Some((user, system, idle)) = self.cpu_split {
                    cards.push(
                        self.stat_card("System", format!("{:.1}%", system), red)
                            .into_any_element(),
                    );
                    cards.push(
                        self.stat_card("User", format!("{:.1}%", user), blue)
                            .into_any_element(),
                    );
                    cards.push(
                        self.stat_card("Idle", format!("{:.1}%", idle), mac::text_secondary())
                            .into_any_element(),
                    );
                }
                (cards, self.history.cpu.as_slice(), blue)
            }
            Tab::Memory => {
                let cards = vec![
                    self.stat_card(
                        "Memory Used",
                        format!(
                            "{} / {}",
                            format_mem(self.agg.mem_used),
                            format_mem(self.agg.mem_total)
                        ),
                        green,
                    )
                    .into_any_element(),
                    self.stat_card("Available", format_mem(self.agg.mem_available), mac::text())
                        .into_any_element(),
                    self.stat_card(
                        "Swap",
                        format!(
                            "{} / {}",
                            format_mem(self.agg.swap_used),
                            format_mem(self.agg.swap_total)
                        ),
                        mac::text(),
                    )
                    .into_any_element(),
                ];
                (cards, self.history.mem.as_slice(), green)
            }
            Tab::Energy => {
                let cards = vec![
                    self.stat_card(
                        "Energy Impact",
                        format!("{:.1}", self.agg.energy_total),
                        orange,
                    )
                    .into_any_element(),
                    self.stat_card(
                        "CPU Load",
                        format!("{:.1}%", self.agg.cpu_total),
                        mac::text(),
                    )
                    .into_any_element(),
                ];
                (cards, self.history.energy.as_slice(), orange)
            }
            Tab::Disk => {
                let cards = vec![
                    self.stat_card("Reads", format_rate(self.agg.disk_read_rate), purple)
                        .into_any_element(),
                    self.stat_card("Writes", format_rate(self.agg.disk_write_rate), purple)
                        .into_any_element(),
                ];
                (cards, self.history.disk.as_slice(), purple)
            }
            Tab::Network => {
                let cards = vec![
                    self.stat_card("Receiving", format_rate(self.agg.net_recv_rate), teal)
                        .into_any_element(),
                    self.stat_card("Sending", format_rate(self.agg.net_sent_rate), teal)
                        .into_any_element(),
                ];
                (cards, self.history.net.as_slice(), teal)
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
                matches!(self.tab, Tab::Cpu) && !self.agg.per_core.is_empty(),
                |el| el.child(self.render_core_bars()),
            )
            .when(
                matches!(self.tab, Tab::Memory) && self.agg.mem_total > 0,
                |el| el.child(self.render_mem_pressure()),
            )
    }

    /// The macOS Memory Pressure indicator: a colored bar whose fill tracks the
    /// real used/total ratio, in the green / yellow / red zones Activity Monitor
    /// uses. Real data — `sysinfo` used memory (active + wired + compressed).
    fn render_mem_pressure(&self) -> impl IntoElement {
        let frac = (self.agg.mem_used as f32 / self.agg.mem_total as f32).clamp(0.0, 1.0);
        let (color, label): (gpui::Hsla, &str) = if frac < 0.60 {
            (gpui::rgb(0x28b463).into(), "Normal")
        } else if frac < 0.80 {
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
                            .w(gpui::relative(frac))
                            .rounded(px(5.0))
                            .bg(color),
                    ),
            )
    }

    fn render_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let active = self.tab;
        let selected = Tab::ALL.iter().position(|tab| *tab == active).unwrap_or(0);
        let group = Tabs::new("activity-tabs", Tab::ALL.map(Tab::label))
            .selected(selected)
            .on_change(cx.listener(|this, index: &usize, _, cx| {
                this.select_tab(Tab::ALL[*index], cx);
            }));

        let has_sel = self.selected_proc(cx).is_some();

        div()
            .h_flex()
            .items_center()
            .justify_between()
            .gap_3()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(mac::separator())
            .child(group)
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("quit", "Quit")
                            .disabled(!has_sel)
                            .on_click(cx.listener(|this, _, _, cx| this.request_kill(false, cx))),
                    )
                    .child(
                        Button::new("force-quit", "Force Quit")
                            .destructive()
                            .disabled(!has_sel)
                            .on_click(cx.listener(|this, _, _, cx| this.request_kill(true, cx))),
                    )
                    .when(self.tab.has_process_table(), |el| {
                        el.child(
                            Button::new("columns", "Columns")
                                .selected(self.cols_menu_open)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cols_menu_open = !this.cols_menu_open;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        div()
                            .w(px(220.0))
                            .child(SearchField::new(&self.search).small()),
                    ),
            )
    }

    fn render_confirm(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        use rmac_ui::DialogButtonKind::{Destructive, Normal, Primary};
        let p = self.pending_kill.clone()?;
        let verb = p.kind.label();
        let body = format!(
            "Do you want to {} the process \u{201c}{}\u{201d} (PID {})?",
            verb.to_lowercase(),
            p.process.name,
            p.process.pid
        );
        let confirm_kind = if p.kind == process_action::ActionKind::ForceQuit {
            Destructive
        } else {
            Primary
        };
        Some(rmac_ui::alert(
            format!("{verb} Process"),
            body,
            vec![
                rmac_ui::dialog_button("kill-cancel", "Cancel", Normal)
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_kill(cx)))
                    .into_any_element(),
                rmac_ui::dialog_button("kill-confirm", verb, confirm_kind)
                    .on_click(cx.listener(|this, _, _, cx| this.confirm_kill(cx)))
                    .into_any_element(),
            ],
        ))
    }

    /// The double-click process inspector — a detail panel of real `sysinfo` data.
    fn render_inspector(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let pid = self.inspect_pid?;
        let state = self.table.read(cx);
        let d = state.delegate();
        let row = d
            .rows
            .iter()
            .chain(d.all_rows.iter())
            .find(|r| r.pid == pid)?;
        let path = d
            .system
            .process(Pid::from_u32(pid))
            .and_then(|p| p.exe().map(|e| e.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "—".into());

        let info_row = |label: &str, value: String| {
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .gap_4()
                .py_1p5()
                .border_b_1()
                .border_color(mac::separator())
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text_secondary())
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .font_weight(mac::MEDIUM)
                        .text_color(mac::text())
                        .child(value),
                )
        };

        let dialog = div()
            .v_flex()
            .gap_1()
            .w(px(420.0))
            .p_5()
            .rounded(px(12.0))
            .bg(mac::window())
            .border_1()
            .border_color(mac::separator())
            .shadow_lg()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .pb_2()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(16.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text())
                            .child(row.name.clone()),
                    )
                    .child(Button::new("inspect-close", "Done").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.inspect_pid = None;
                            cx.notify();
                        },
                    ))),
            )
            .child(info_row("Process ID (PID)", row.pid.to_string()))
            .child(info_row(
                "Parent PID",
                row.ppid
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "—".into()),
            ))
            .child(info_row("User", row.user.to_string()))
            .child(info_row("Status", row.status.to_string()))
            .child(info_row("% CPU", format!("{:.1}", row.cpu)))
            .child(info_row("Memory", format_mem(row.mem)))
            .child(info_row("Virtual Memory", format_mem(row.vmem)))
            .child(info_row("Disk I/O", format_mem(row.disk)))
            .child(info_row("Run Time", format_duration(row.run_time)))
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .pt_2()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(mac::text_secondary())
                            .child("Path"),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .text_color(mac::text())
                            .child(path),
                    ),
            );

        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(mac::scrim())
                .child(dialog),
        )
    }
}

impl Render for MonitorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let persistence_error = self.persistence_error.clone();
        let process_feedback = self.process_action_feedback.clone();
        div()
            .track_focus(&self.focus)
            .key_context("ActivityMonitor")
            .on_action(cx.listener(|this, _: &QuitProcess, _, cx| this.request_kill(false, cx)))
            .on_action(cx.listener(|this, _: &ForceQuitProcess, _, cx| this.request_kill(true, cx)))
            .on_action(
                cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ConfirmKill, _, cx| this.confirm_kill(cx)))
            .on_action(cx.listener(|this, _: &CancelKill, _, cx| this.cancel_kill(cx)))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            .size_full()
            .v_flex()
            .bg(mac::window())
            .text_color(mac::text())
            .child(rmac_ui::title_bar("System Monitor"))
            .child(self.render_toolbar(cx))
            .when_some(persistence_error, |monitor, message| {
                monitor.child(
                    div()
                        .id("persistence-error")
                        .h(px(34.0))
                        .flex_none()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(mac::error_background())
                        .border_b_1()
                        .border_color(mac::error_border())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::danger())
                        .cursor_pointer()
                        .child(div().flex_1().child(message))
                        .child("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.persistence_error = None;
                            cx.notify();
                        })),
                )
            })
            .when_some(process_feedback, |monitor, feedback| {
                let (background, border, text) = if feedback.success {
                    (mac::accent_subtle(), mac::accent_border(), mac::text())
                } else {
                    (mac::error_background(), mac::error_border(), mac::danger())
                };
                monitor.child(
                    div()
                        .id("process-action-feedback")
                        .min_h(px(52.0))
                        .flex_none()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .bg(background)
                        .border_b_1()
                        .border_color(border)
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(text)
                        .cursor_pointer()
                        .child(
                            div()
                                .v_flex()
                                .flex_1()
                                .gap_1()
                                .child(div().font_weight(mac::SEMIBOLD).child(feedback.title))
                                .child(feedback.detail),
                        )
                        .child("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.process_action_feedback = None;
                            cx.notify();
                        })),
                )
            })
            .child(self.render_summary(cx))
            .when(self.tab.has_process_table(), |this| {
                this.child(
                    // The live table fills the rest
                    div()
                        .flex_1()
                        .px_4()
                        .pb_4()
                        .child(Table::new(&self.table).stripe(true).bordered(true)),
                )
            })
            .when(!self.tab.has_process_table(), |this| {
                this.child(self.render_network_pane())
            })
            .when(
                self.cols_menu_open && self.tab.has_process_table(),
                |this| this.child(self.render_columns_menu(cx)),
            )
            .children(self.render_confirm(cx))
            .children(self.render_inspector(cx))
    }
}

fn main() {
    rmac_ui::boot_app(
        rmac_ui::app_id::SYSTEM_MONITOR,
        "System Monitor",
        1040.0,
        680.0,
        |window, cx| {
            let view = MonitorView::new(window, cx);
            cx.bind_keys([
                gpui::KeyBinding::new(
                    rmac_ui::shortcuts::FIND.keystroke,
                    FocusSearch,
                    Some("ActivityMonitor"),
                ),
                gpui::KeyBinding::new(
                    rmac_ui::shortcuts::DELETE.keystroke,
                    QuitProcess,
                    Some("ActivityMonitor"),
                ),
                gpui::KeyBinding::new(
                    rmac_ui::shortcuts::FORCE_DELETE.keystroke,
                    ForceQuitProcess,
                    Some("ActivityMonitor"),
                ),
                gpui::KeyBinding::new(
                    rmac_ui::shortcuts::ENTER.keystroke,
                    ConfirmKill,
                    Some("ActivityMonitor"),
                ),
                gpui::KeyBinding::new(
                    rmac_ui::shortcuts::ESCAPE.keystroke,
                    CancelKill,
                    Some("ActivityMonitor"),
                ),
            ]);
            window.focus(&view.focus);
            view
        },
    );
}
