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
use crate::floating_windows::MetricWindowKind;
use crate::metrics::{Tab, REFRESH_SECS};
use crate::process_table::{resync_selection, ProcessTableDelegate};
use crate::sampling::Sampler;
use crate::signal_picker::NamedSignal;
use crate::view_filter::ViewFilter;
use crate::{floating_windows, process_action, process_signal, quit_and_keep_windows};

/// Overall host CPU usage as a 0.0..=1.0 fraction, from the sampler's
/// (user%, system%, idle%) split — for the View ▸ Dock Icon CPU Usage tile.
fn cpu_fraction((user, system, _idle): (f32, f32, f32)) -> f64 {
    (f64::from(user) + f64::from(system)) / 100.0
}

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
    pending_kill: process_action::Confirmation,
    process_action_feedback: Option<process_action::Feedback>,
    /// Whether the column chooser dropdown is open.
    cols_menu_open: bool,
    persistence_error: Option<SharedString>,
    /// PID whose detail inspector is open (double-click a row).
    inspect_pid: Option<u32>,
    /// View ▸ Show Deltas for Process, ⌥⌘J: the inspector adds rows showing
    /// the change in % CPU and Memory since `delta_baseline` was captured.
    show_deltas: bool,
    /// (pid, % CPU, Memory) sampled when deltas started being shown for
    /// this process; cleared when deltas are turned off or a different
    /// process is inspected while they're on.
    delta_baseline: Option<(u32, f32, u64)>,
    /// View ▸ Dock Icon.
    dock_icon_mode: crate::dock_icon::DockIconMode,
    /// Whether the toolbar's search circle has expanded into a field.
    search_open: bool,
    /// Whether the View filter dropdown (MON-03) is open.
    pub(crate) filter_menu_open: bool,
    refresh_seconds: u64,
    refresh_wake: async_channel::Sender<()>,
    /// View ▸ Send Signal to Process… (MON-10/14): the process the sheet
    /// is open for, captured at open time the same way `pending_kill` is.
    signal_sheet: Option<process_action::ProcessIdentity>,
    signal_feedback: Option<process_action::Feedback>,
    /// View ▸ Sample Process (MON-10/14): whether a sample is currently
    /// being collected off the UI thread.
    sample_in_progress: bool,
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
        // for the target PID — otherwise a background refresh could re-point
        // the highlighted row at a different process before a kill is requested.
        cx.subscribe(&table, |this, table, event: &TableEvent, cx| match event {
            TableEvent::SelectRow(row_ix) => {
                let row_ix = *row_ix;
                table.update(cx, |state, _| {
                    let pid = state.delegate().rows.get(row_ix).map(|r| r.pid);
                    state.delegate_mut().set_selected_pid(pid);
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

        let (wake, events) = async_channel::bounded(1);
        let mut view = Self {
            table,
            search,
            focus: cx.focus_handle(),
            tab: Tab::Cpu,
            sampler: Sampler::new(),
            pending_kill: process_action::Confirmation::default(),
            process_action_feedback: None,
            cols_menu_open: false,
            persistence_error,
            inspect_pid: None,
            show_deltas: false,
            delta_baseline: None,
            dock_icon_mode: crate::dock_icon::DockIconMode::default(),
            search_open: false,
            filter_menu_open: false,
            refresh_seconds: REFRESH_SECS as u64,
            refresh_wake: wake.clone(),
            signal_sheet: None,
            signal_feedback: None,
            sample_in_progress: false,
        };
        view.refresh(cx);

        // An inactive window has no graph to update and no timer to run.
        cx.observe_window_activation(window, move |view, window, cx| {
            let _ = wake.try_send(());
            if window.is_window_active() {
                view.refresh(cx);
                cx.notify();
            }
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| loop {
            let (active, seconds) = this
                .update_in(cx, |view, window, _| {
                    (window.is_window_active(), view.refresh_seconds)
                })
                .unwrap_or((false, REFRESH_SECS as u64));
            let timer_expired = futures_lite::future::race(
                async {
                    if active {
                        cx.background_executor()
                            .timer(Duration::from_secs(seconds))
                            .await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                    true
                },
                async {
                    let _ = events.recv().await;
                    false
                },
            )
            .await;
            if this
                .update_in(cx, |view, window, cx| {
                    if timer_expired && window.is_window_active() {
                        view.refresh(cx);
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
        })
        .detach();

        // Application ▸ Quit and Keep Windows (MON-16): reopen exactly the
        // floating metric windows that were open when that action last ran,
        // once. Deferred (rather than opened here directly) since this
        // constructor itself still runs inside `cx.new`'s entity-creation
        // closure, matching how every other secondary-window `show` call in
        // this codebase is invoked from `cx.defer`.
        let restored = quit_and_keep_windows::take_saved();
        if !restored.is_empty() {
            let entity = cx.entity();
            cx.defer(move |cx| {
                for kind in restored {
                    floating_windows::show(kind, entity.clone(), cx);
                }
            });
        }

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
        if self.dock_icon_mode != crate::dock_icon::DockIconMode::Application {
            crate::dock_icon::publish(
                self.dock_icon_mode,
                self.sampler.cpu_split.map(cpu_fraction),
            );
        }
    }

    /// View ▸ Dock Icon.
    fn set_dock_icon_mode(&mut self, mode: crate::dock_icon::DockIconMode, cx: &mut Context<Self>) {
        self.dock_icon_mode = mode;
        crate::dock_icon::publish(mode, self.sampler.cpu_split.map(cpu_fraction));
        cx.notify();
    }

    fn select_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.cols_menu_open = false;
        self.filter_menu_open = false;
        self.table.update(cx, |state, cx| {
            let d = state.delegate_mut();
            // MON-02: each tab shows its own remembered column set.
            d.activate_tab(tab);
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

    /// The menu bar's live state for the key window. Process commands are
    /// greyed out without a selection, while view choices carry checkmarks.
    pub(super) fn publish_menu_state(&self, window: &Window, cx: &mut Context<Self>) {
        rmac_ui::set_menu_label(
            "activity_monitor::EnterFullScreen",
            if window.is_fullscreen() {
                "Exit Full Screen"
            } else {
                "Enter Full Screen"
            },
            cx,
        );
        let has_selection = self.selected_proc(cx).is_some();
        rmac_ui::set_menu_enabled("activity_monitor::QuitProcess", has_selection, cx);
        rmac_ui::set_menu_enabled("activity_monitor::InspectProcess", has_selection, cx);
        rmac_ui::set_menu_enabled("activity_monitor::SendSignalToProcess", has_selection, cx);
        rmac_ui::set_menu_enabled("activity_monitor::SampleProcess", has_selection, cx);
        let has_matches = !self.search.read(cx).value().is_empty()
            && !self.table.read(cx).delegate().rows.is_empty();
        rmac_ui::set_menu_enabled("activity_monitor::FindNext", has_matches, cx);
        rmac_ui::set_menu_enabled("activity_monitor::FindPrevious", has_matches, cx);
        let has_text_selection = !self.search.read(cx).selected_value().is_empty();
        rmac_ui::set_menu_enabled(
            "activity_monitor::UseSelectionForFind",
            has_selection || has_text_selection,
            cx,
        );
        rmac_ui::set_menu_enabled("activity_monitor::JumpToSelection", has_selection, cx);
        for (action, filter) in [
            ("activity_monitor::ShowAllProcesses", ViewFilter::All),
            ("activity_monitor::ShowMyProcesses", ViewFilter::MyProcesses),
            (
                "activity_monitor::ShowSystemProcesses",
                ViewFilter::SystemProcesses,
            ),
            (
                "activity_monitor::ShowOtherUsersProcesses",
                ViewFilter::OtherUsersProcesses,
            ),
            (
                "activity_monitor::ShowActiveProcesses",
                ViewFilter::ActiveProcesses,
            ),
            (
                "activity_monitor::ShowInactiveProcesses",
                ViewFilter::InactiveProcesses,
            ),
            (
                "activity_monitor::ShowSelectedProcesses",
                ViewFilter::SelectedProcesses,
            ),
        ] {
            rmac_ui::set_menu_checked(action, self.view_filter(cx) == filter, cx);
        }
        rmac_ui::set_menu_enabled("activity_monitor::ShowSelectedProcesses", has_selection, cx);
        for (action, seconds) in [
            ("activity_monitor::RefreshEverySecond", 1),
            ("activity_monitor::RefreshEveryTwoSeconds", 2),
            ("activity_monitor::RefreshEveryFiveSeconds", 5),
        ] {
            rmac_ui::set_menu_checked(action, self.refresh_seconds == seconds, cx);
        }
        let visible = self.table.read(cx).delegate().visible.clone();
        for (action, column) in [
            ("activity_monitor::TogglePidColumn", ColKey::Pid),
            ("activity_monitor::ToggleUserColumn", ColKey::User),
            ("activity_monitor::ToggleCpuColumn", ColKey::Cpu),
            ("activity_monitor::ToggleThreadsColumn", ColKey::Threads),
            ("activity_monitor::ToggleMemoryColumn", ColKey::Mem),
            ("activity_monitor::ToggleEnergyColumn", ColKey::Energy),
        ] {
            rmac_ui::set_menu_checked(action, visible.contains(&column), cx);
        }
        rmac_ui::set_menu_checked(
            "activity_monitor::ShowDeltasForProcess",
            self.show_deltas,
            cx,
        );
        rmac_ui::set_menu_checked(
            "activity_monitor::SetDockIconApplication",
            self.dock_icon_mode == crate::dock_icon::DockIconMode::Application,
            cx,
        );
        rmac_ui::set_menu_checked(
            "activity_monitor::SetDockIconCpuUsage",
            self.dock_icon_mode == crate::dock_icon::DockIconMode::CpuUsage,
            cx,
        );
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

    fn request_kill(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        // Capture whatever row is highlighted right now (covers keyboard nav,
        // which moves the table's selected row without touching `selected_pid`).
        self.table.update(cx, |state, _| {
            if let Some(pid) = state
                .selected_row()
                .and_then(|ix| state.delegate().rows.get(ix))
                .map(|r| r.pid)
            {
                state.delegate_mut().set_selected_pid(Some(pid));
            }
        });
        if let Some(process) = self.selected_proc(cx) {
            self.pending_kill.request(process_action::Request {
                process,
                kind: if force {
                    process_action::ActionKind::ForceQuit
                } else {
                    process_action::ActionKind::Quit
                },
            });
            self.process_action_feedback = None;
            self.cols_menu_open = false;
            // The process table's own "DataTable" key context binds Escape
            // to clear its row selection, which is a deeper context than
            // this view's root and so wins the dispatch -- move focus off
            // the table so Escape/Return resolve at the confirmation dialog
            // instead (see docs/keyboard-audit.md).
            window.focus(&self.focus, cx);
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
            || self.signal_sheet.take().is_some()
            || std::mem::take(&mut self.cols_menu_open)
            || std::mem::take(&mut self.filter_menu_open)
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
        if let Some(request) = self.pending_kill.current_mut() {
            request.kind = process_action::ActionKind::ForceQuit;
        }
        self.confirm_kill(cx);
    }

    /// Open the inspector for the highlighted process (toolbar ⓘ).
    fn inspect_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(process) = self.selected_proc(cx) {
            self.inspect_pid = Some(process.pid);
            self.cols_menu_open = false;
            if self.show_deltas {
                self.recapture_delta_baseline(process.pid, cx);
            }
            cx.notify();
        }
    }

    /// (pid, % CPU, Memory) for `pid` right now, the Show Deltas for
    /// Process baseline every later inspector render compares against.
    fn recapture_delta_baseline(&mut self, pid: u32, cx: &Context<Self>) {
        let state = self.table.read(cx);
        let delegate = state.delegate();
        self.delta_baseline = delegate
            .rows
            .iter()
            .chain(delegate.all_rows.iter())
            .find(|row| row.pid == pid)
            .map(|row| (row.pid, row.cpu, row.mem));
    }

    /// View ▸ Show Deltas for Process, ⌥⌘J.
    fn toggle_show_deltas(&mut self, cx: &mut Context<Self>) {
        self.show_deltas = !self.show_deltas;
        if self.show_deltas {
            if let Some(pid) = self.inspect_pid {
                self.recapture_delta_baseline(pid, cx);
            }
        } else {
            self.delta_baseline = None;
        }
        cx.notify();
    }

    /// Toggle the column chooser (toolbar ⋯). A no-op when the current tab
    /// has no process table, matching the button's own disabled state.
    fn toggle_columns_menu(&mut self, cx: &mut Context<Self>) {
        if self.tab.has_process_table() {
            self.cols_menu_open = !self.cols_menu_open;
            self.filter_menu_open = false;
            cx.notify();
        }
    }

    /// The View filter (MON-03) currently applied to the process table.
    pub(crate) fn view_filter(&self, cx: &Context<Self>) -> ViewFilter {
        self.table.read(cx).delegate().view_filter
    }

    /// Toggle the View filter dropdown under the window subtitle.
    pub(crate) fn toggle_filter_menu(&mut self, cx: &mut Context<Self>) {
        self.filter_menu_open = !self.filter_menu_open;
        self.cols_menu_open = false;
        cx.notify();
    }

    /// Apply a new View filter (MON-03) and close the dropdown.
    pub(crate) fn set_view_filter(&mut self, filter: ViewFilter, cx: &mut Context<Self>) {
        self.filter_menu_open = false;
        self.table.update(cx, |state, cx| {
            state.delegate_mut().set_view_filter(filter);
            resync_selection(state, cx);
            state.refresh(cx);
        });
        cx.notify();
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = true;
        self.search.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    fn find_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.search.read(cx).value().is_empty() {
            return;
        }
        self.table.update(cx, |state, cx| {
            let len = state.delegate().rows.len();
            if len == 0 {
                return;
            }
            let next = match state.selected_row() {
                Some(index) if forward => (index + 1) % len,
                Some(index) => (index + len - 1) % len,
                None if forward => 0,
                None => len - 1,
            };
            let pid = state.delegate().rows[next].pid;
            state.delegate_mut().set_selected_pid(Some(pid));
            state.set_selected_row(next, cx);
        });
        cx.notify();
    }

    /// Search for the name of the selected process, keeping the selected row
    /// visible as the list narrows. This is the table's selection equivalent
    /// of Edit ▸ Find ▸ Use Selection for Find in a text view.
    fn use_selection_for_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected_text = self.search.read(cx).selected_value().to_string();
        let query = if selected_text.is_empty() {
            self.selected_proc(cx).map(|process| process.name)
        } else {
            Some(selected_text)
        };
        let Some(query) = query else {
            return;
        };
        self.search_open = true;
        self.search
            .update(cx, |search, cx| search.set_value(query, window, cx));
        cx.notify();
    }

    /// Scroll the process table back to its selected row.
    fn jump_to_selection(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |state, cx| {
            let Some(pid) = state.delegate().selected_pid else {
                return;
            };
            if let Some(row) = state
                .delegate()
                .rows
                .iter()
                .position(|item| item.pid == pid)
            {
                state.set_selected_row(row, cx);
            }
        });
    }

    fn clear_cpu_history(&mut self, cx: &mut Context<Self>) {
        self.sampler.history.cpu_user.clear();
        self.sampler.history.cpu_system.clear();
        cx.notify();
    }

    fn set_refresh_seconds(&mut self, seconds: u64, cx: &mut Context<Self>) {
        if self.refresh_seconds != seconds {
            self.refresh_seconds = seconds;
            let _ = self.refresh_wake.try_send(());
            cx.notify();
        }
    }

    /// Set the search query from an AT-SPI `SetValue`/`ReplaceSelectedText`
    /// request, the same way `crates/launcher-app` handles Spotlight's query
    /// field. `apply_filter` runs from the existing `cx.observe(&search, ..)`
    /// once the field's value changes, so no extra wiring is needed here.
    pub(crate) fn set_search_from_assistive_technology(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_open = true;
        self.search
            .update(cx, |state, cx| state.set_value(text, window, cx));
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

    // -- Accessors for the floating metric windows (`floating_windows.rs`),
    // which live outside this module's tree and so cannot reach private
    // fields directly (see that module's doc comment on `bar_graph`). --

    pub(crate) fn cpu_split(&self) -> Option<(f32, f32, f32)> {
        self.sampler.cpu_split
    }

    pub(crate) fn cpu_history(&self) -> (&[f32], &[f32]) {
        (
            &self.sampler.history.cpu_user,
            &self.sampler.history.cpu_system,
        )
    }

    pub(crate) fn gpu_reading(&self) -> crate::gpu_stats::GpuReading {
        self.sampler.gpu_reading
    }

    pub(crate) fn gpu_history(&self) -> &[f32] {
        &self.sampler.history.gpu
    }

    /// Window ▸ CPU Usage / CPU History / GPU History.
    pub(crate) fn open_metric_window(&mut self, kind: MetricWindowKind, cx: &mut Context<Self>) {
        let view = cx.entity();
        floating_windows::show(kind, view, cx);
    }

    /// Application ▸ Quit and Keep Windows (MON-16, MON-MENU-001): persist
    /// which floating metric windows are open right now, then actually
    /// quit. An ordinary Close/⌘Q never calls this, so an ordinary next
    /// launch restores nothing — only this action opts in.
    pub(crate) fn quit_and_keep_open_windows(&mut self, cx: &mut Context<Self>) {
        let open = floating_windows::open_kinds(cx);
        quit_and_keep_windows::save(&open);
        cx.quit();
    }

    /// View ▸ Send Signal to Process… (MON-10/14, MON-MENU-032): opens the
    /// sheet for whichever process is highlighted right now.
    pub(crate) fn open_signal_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.table.update(cx, |state, _| {
            if let Some(pid) = state
                .selected_row()
                .and_then(|ix| state.delegate().rows.get(ix))
                .map(|r| r.pid)
            {
                state.delegate_mut().set_selected_pid(Some(pid));
            }
        });
        if let Some(process) = self.selected_proc(cx) {
            self.signal_sheet = Some(process);
            self.signal_feedback = None;
            self.pending_kill.take();
            self.inspect_pid = None;
            self.cols_menu_open = false;
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    pub(crate) fn cancel_signal_sheet(&mut self, cx: &mut Context<Self>) {
        if self.signal_sheet.take().is_some() {
            cx.notify();
        }
    }

    /// Send `signal` to the process the sheet opened for, with the same
    /// identity preflight Quit/Force Quit use. A permission failure for a
    /// process owned by another user escalates once through `pkexec`
    /// (MON-10/14: "polkit for other users' processes") rather than
    /// failing closed outright.
    pub(crate) fn send_named_signal(&mut self, signal: NamedSignal, cx: &mut Context<Self>) {
        let Some(identity) = self.signal_sheet.take() else {
            return;
        };
        self.signal_feedback = None;
        let (outcome, owner) = self.table.update(cx, |state, cx| {
            let delegate = state.delegate_mut();
            let pid = Pid::from_u32(identity.pid);
            delegate.system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                true,
                ProcessRefreshKind::nothing(),
            );
            let observed =
                delegate
                    .system
                    .process(pid)
                    .map(|process| process_action::ProcessIdentity {
                        pid: process.pid().as_u32(),
                        start_time: process.start_time(),
                        name: process.name().to_string_lossy().into_owned(),
                    });
            let owner = delegate
                .rows
                .iter()
                .chain(delegate.all_rows.iter())
                .find(|row| row.pid == identity.pid)
                .map(|row| row.user.to_string());
            let outcome = match process_action::identity_preflight(&identity, observed.as_ref()) {
                process_action::Preflight::Missing => process_action::Outcome::Missing,
                process_action::Preflight::Replaced => process_action::Outcome::Replaced,
                process_action::Preflight::Current => {
                    match process_signal::ProcessHandle::open(identity.pid) {
                        Ok(handle) => process_signal_outcome(handle.send(
                            process_signal::SignalKind::Named(signal),
                            || {
                                delegate
                                    .system
                                    .process(pid)
                                    .and_then(|process| process.kill_with(signal.sysinfo_signal()))
                            },
                        )),
                        Err(outcome) => process_signal_outcome(outcome),
                    }
                }
            };
            if matches!(
                outcome,
                process_action::Outcome::Missing | process_action::Outcome::Replaced
            ) {
                delegate.all_rows.retain(|row| row.pid != identity.pid);
                delegate.apply_view();
                delegate.selected_pid = None;
                resync_selection(state, cx);
                state.refresh(cx);
            }
            (outcome, owner)
        });

        let current_user = std::env::var("USER").ok();
        let needs_escalation = outcome == process_action::Outcome::Rejected
            && owner
                .as_ref()
                .is_some_and(|owner| current_user.as_deref() != Some(owner.as_str()));

        if needs_escalation {
            let owner = owner.unwrap_or_default();
            self.signal_feedback = Some(process_action::Feedback {
                success: false,
                title: "Requesting administrator privileges…".into(),
                detail: format!(
                    "{} (PID {}) is owned by {owner}. Asking for permission to send {}.",
                    identity.name,
                    identity.pid,
                    signal.label()
                ),
            });
            cx.notify();
            let pid = identity.pid;
            let name = identity.name.clone();
            let signal_number = signal.number();
            let signal_label = signal.label();
            cx.spawn(async move |this, cx| {
                let outcome = cx
                    .background_executor()
                    .spawn(async move { crate::signal_escalation::escalate(pid, signal_number) })
                    .await;
                let feedback =
                    process_action::signal_escalation_feedback(&name, pid, signal_label, outcome);
                let _ = this.update(cx, |view, cx| {
                    view.signal_feedback = Some(feedback);
                    cx.notify();
                });
            })
            .detach();
        } else {
            self.signal_feedback = Some(process_action::signal_feedback(
                &identity,
                signal.label(),
                outcome,
            ));
            cx.notify();
        }
    }

    /// View ▸ Sample Process (MON-10/14, MON-MENU-031): a real stack
    /// sample of the highlighted process, collected off the UI thread and
    /// shown in its own window (`sample_window.rs`) once ready.
    pub(crate) fn sample_selected_process(&mut self, cx: &mut Context<Self>) {
        if self.sample_in_progress {
            return;
        }
        let Some(process) = self.selected_proc(cx) else {
            return;
        };
        self.sample_in_progress = true;
        cx.notify();
        let pid = process.pid;
        let name = process.name.clone();
        cx.spawn(async move |this, cx| {
            let report = cx
                .background_executor()
                .spawn(async move { crate::sampling_report::collect(pid, name) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.sample_in_progress = false;
                cx.notify();
            });
            cx.update(|cx| crate::sample_window::show(report, cx));
        })
        .detach();
    }
}
