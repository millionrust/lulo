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
mod view_render;

use std::time::Duration;

use gpui::{AppContext as _, Context, Entity, SharedString, Window};
use rmac_ui::{InputState, TableEvent, TableState};
use sysinfo::{Networks, Pid, ProcessRefreshKind, ProcessesToUpdate, Signal};

use columns::{default_visible as default_visible_cols, load as load_visible_cols};
use columns::{save as save_visible_cols, ColKey};
use metrics::{Aggregates, History, NetIface, Tab, REFRESH_SECS};
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
