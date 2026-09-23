//! System Monitor session controller.

mod render;
mod responsive_layout;

use std::time::Duration;

use gpui::{AppContext as _, Context, Entity, SharedString, Window};
use rmac_ui::{InputState, TableEvent, TableState};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, Signal};

use crate::columns::{
    default_visible as default_visible_cols, load as load_visible_cols, save as save_visible_cols,
    ColKey,
};
use crate::metrics::Tab;
use crate::process_table::{resync_selection, ProcessTableDelegate};
use crate::sampling::Sampler;
use crate::{process_action, process_signal};

fn process_signal_outcome(outcome: process_signal::SignalOutcome) -> process_action::Outcome {
    match outcome {
        process_signal::SignalOutcome::Delivered => process_action::Outcome::Delivered,
        process_signal::SignalOutcome::Missing => process_action::Outcome::Missing,
        process_signal::SignalOutcome::Unsupported => process_action::Outcome::Unsupported,
        process_signal::SignalOutcome::Rejected => process_action::Outcome::Rejected,
    }
}

/// Root view: a tabbed summary + the live process table.
pub(crate) struct MonitorView {
    table: Entity<TableState<ProcessTableDelegate>>,
    search: Entity<InputState>,
    pub(crate) focus: gpui::FocusHandle,
    tab: Tab,
    sampler: Sampler,
    pending_kill: Option<process_action::Request>,
    process_action_feedback: Option<process_action::Feedback>,
    /// Whether the column chooser dropdown is open.
    cols_menu_open: bool,
    persistence_error: Option<SharedString>,
    /// PID whose detail inspector is open (double-click a row).
    inspect_pid: Option<u32>,
    /// Whether the toolbar's search circle has expanded into a field.
    search_open: bool,
}

impl MonitorView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            tab: Tab::Cpu,
            sampler: Sampler::new(),
            pending_kill: None,
            process_action_feedback: None,
            cols_menu_open: false,
            persistence_error,
            inspect_pid: None,
            search_open: false,
        };
        view.refresh(cx);

        // Auto-refresh loop — every 2s, off the render path.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| loop {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let Some(this) = this.upgrade() else { break };
            cx.update_entity(&this, |view: &mut MonitorView, cx| {
                view.refresh(cx);
                cx.notify();
            });
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
        self.sampler.refresh(&self.table, cx);
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
        // Escape also dismisses the inspector, the column chooser and an
        // empty search field (collapsing it back to the toolbar circle).
        let empty_search = self.search_open && self.search.read(cx).value().is_empty();
        if self.pending_kill.take().is_some()
            || self.inspect_pid.take().is_some()
            || std::mem::take(&mut self.cols_menu_open)
        {
            cx.notify();
        } else if empty_search {
            self.search_open = false;
            cx.notify();
        }
    }

    /// Force-quit the process a pending Quit confirmation names, from the
    /// alert's Force Quit button.
    fn confirm_force_quit(&mut self, cx: &mut Context<Self>) {
        if let Some(request) = self.pending_kill.as_mut() {
            request.kind = process_action::ActionKind::ForceQuit;
        }
        self.confirm_kill(cx);
    }

    /// Open the inspector for the highlighted process (toolbar ⓘ).
    fn inspect_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(process) = self.selected_proc(cx) {
            self.inspect_pid = Some(process.pid);
            self.cols_menu_open = false;
            cx.notify();
        }
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = true;
        self.search.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
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
