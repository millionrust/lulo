use std::cmp::Ordering;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, AccessibleAction, App, Context, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement, Role, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    Window,
};
use gpui_component::menu::PopupMenu;
use rmac_activity_monitor::accessibility::{
    self, ProcessColumn, ProcessRowSemantics, ProcessTableAccessibilitySnapshot, SortDirection,
};
use rmac_ui::{Column, ColumnSort, TableDelegate, TableState};
use sysinfo::{
    Process, ProcessRefreshKind, ProcessesToUpdate, System, ThreadKind, UpdateKind, Users,
};

use crate::columns::{self, ColKey};
use crate::metrics::{format_duration, format_mem, Tab};
use crate::view_filter::ViewFilter;
use crate::{ForceQuitProcess, QuitProcess};

/// True for a Linux thread-group leader — a real process, in the sense the
/// Mac's Activity Monitor means it (one row per running program). `sysinfo`
/// 0.33 also walks `/proc/<pid>/task/<tid>` and returns each *other* thread
/// as its own entry with `thread_kind() == Some(ThreadKind::Userland)`; a
/// naive `system.processes()` therefore lists every thread as a process
/// (MON-01), and any sum over that map double-counts CPU/memory/disk, since
/// the leader's own `/proc/<pid>/stat` and `/proc/<pid>/io` already report
/// the whole thread group's totals. Kernel threads (`ThreadKind::Kernel`,
/// e.g. `[kworker/0:0]`) are kept: each is its own independent schedulable
/// unit with `pid == tgid`, the same as `ps`/`top` show them.
pub(crate) fn is_thread_group_leader(process: &Process) -> bool {
    is_leader_thread_kind(process.thread_kind())
}

/// The pure predicate behind [`is_thread_group_leader`], taking
/// `Process::thread_kind()`'s result directly so it can be exercised with a
/// fixture table instead of a live `sysinfo::Process` (which has no public
/// constructor — a real `System` refresh is the only other way to get one).
fn is_leader_thread_kind(thread_kind: Option<ThreadKind>) -> bool {
    !matches!(thread_kind, Some(ThreadKind::Userland))
}

/// A thread-group's size: the leader itself, plus every other task
/// `Process::tasks()` found for it (`tasks()` excludes the leader's own
/// tid). MON-01: this is what "Threads" must count — never the number of
/// table rows, which before this fix conflated "Threads" with "Processes".
fn thread_group_size(other_tasks: Option<usize>) -> u32 {
    1 + other_tasks.unwrap_or(0) as u32
}

/// One process-table snapshot. The command search text stays private because it
/// is only an indexing implementation detail; the view consumes the remaining
/// measured fields for summaries, actions, and the inspector.
#[derive(Clone)]
pub(crate) struct ProcRow {
    pub(crate) pid: u32,
    pub(crate) name: SharedString,
    /// Lower-cased command line / executable path, used for search matching only.
    cmd_search: SharedString,
    pub(crate) cpu: f32,
    /// `sysinfo` computes per-process CPU as a delta between two reads; the
    /// very first refresh has no previous reading and always reports 0.0.
    /// `cell_text` shows "—" instead of that false idle reading until a
    /// second sample exists, matching the header CPU figures.
    pub(crate) cpu_ready: bool,
    pub(crate) mem: u64,
    /// Bytes read+written since the last refresh (a per-tick I/O proxy).
    pub(crate) disk: u64,
    /// Energy-impact approximation. macOS's exact figure is proprietary; we
    /// combine the two real energy-relevant signals sysinfo exposes — CPU usage
    /// plus this interval's disk I/O — so it isn't merely a copy of %CPU.
    pub(crate) energy: f32,
    /// Parent process id (real, from sysinfo).
    pub(crate) ppid: Option<u32>,
    /// Owning user name, resolved from the process uid.
    pub(crate) user: SharedString,
    /// Virtual memory size in bytes.
    pub(crate) vmem: u64,
    /// Wall-clock run time in seconds since the process started.
    pub(crate) run_time: u64,
    /// Process start time binds destructive actions across PID reuse.
    pub(crate) start_time: u64,
    /// Process status (Running / Sleeping / …), for sorting + display.
    pub(crate) status: SharedString,
    /// Threads in this process's thread group (the leader itself plus every
    /// other task `sysinfo` found under `/proc/<pid>/task`). See
    /// [`is_thread_group_leader`] for why this is computed on leader rows
    /// only rather than by counting rows.
    pub(crate) threads: u32,
    /// Owning uid, for the MON-03 View filter (My/System/Other Users').
    pub(crate) uid: Option<u32>,
}

impl ProcRow {
    fn cell_text(&self, key: ColKey) -> String {
        match key {
            ColKey::Pid => self.pid.to_string(),
            ColKey::Name => self.name.to_string(),
            ColKey::Cpu => {
                if self.cpu_ready {
                    format!("{:.1}", self.cpu)
                } else {
                    "—".to_string()
                }
            }
            ColKey::Mem => format_mem(self.mem),
            ColKey::Energy => format!("{:.1}", self.energy),
            ColKey::Disk => format_mem(self.disk),
            ColKey::Threads => self.threads.to_string(),
            ColKey::Ppid => self.ppid.map(|pid| pid.to_string()).unwrap_or_default(),
            ColKey::User => self.user.to_string(),
            ColKey::Vmem => format_mem(self.vmem),
            ColKey::RunTime => format_duration(self.run_time),
            ColKey::Status => self.status.to_string(),
        }
    }
}

impl ProcessRowSemantics for ProcRow {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn name(&self) -> &str {
        self.name.as_ref()
    }

    fn cell_text(&self, column: ProcessColumn) -> String {
        self.cell_text(column.into())
    }
}

/// Owns the live `System` handle plus the process-table projection.
///
/// A single `System` instance is refreshed in place so `sysinfo` can compute
/// per-process CPU deltas between ticks. Stable process identity, filtering,
/// sorting, columns, and selection all remain inside this boundary.
pub(crate) struct ProcessTableDelegate {
    pub(crate) system: System,
    /// Full unfiltered snapshot.
    pub(crate) all_rows: Vec<ProcRow>,
    /// Filtered + sorted rows actually shown.
    pub(crate) rows: Vec<ProcRow>,
    /// The visible columns, in display order — a subset of `ColKey::ALL`.
    pub(crate) visible: Vec<ColKey>,
    /// `gpui-component` column descriptors mirroring `visible`.
    columns: Vec<Column>,
    /// uid → username resolution table. Accounts are stable over a monitor
    /// session, so this is loaded once rather than refreshed every two seconds.
    users: Users,
    filter: String,
    /// The column the rows are sorted by — identity-based so it survives the
    /// visible set changing under it.
    pub(crate) sort_key: ColKey,
    pub(crate) sort_asc: bool,
    /// PID is the selection source of truth because row indexes drift whenever
    /// the table is refreshed or sorted.
    pub(crate) selected_pid: Option<u32>,
    /// AT-SPI projection of the currently visible rows (accessibility.rs's
    /// `project_process_table`). Refreshed alongside `apply_view` — the same
    /// data refresh/filter/sort/column-toggle cadence the table already runs
    /// on — and on every selection change, never per render frame.
    pub(crate) accessible: ProcessTableAccessibilitySnapshot,
    /// Counts calls to `refresh()`. Per-process CPU needs two `sysinfo`
    /// reads to compute a delta, so rows built from the first refresh get
    /// `cpu_ready: false`.
    refresh_count: u32,
    /// The tab `visible`/`columns` currently reflect.
    current_tab: Tab,
    /// Each tab's own column set (MON-02), indexed the same as `Tab::ALL`
    /// (5 tabs: CPU, Memory, Energy, Disk, Network), remembered for the rest
    /// of this session as the chooser customizes them. Only `current_tab`'s
    /// slot always mirrors `visible`.
    tab_columns: [Vec<ColKey>; 5],
    /// The View menu's process filter (MON-03).
    pub(crate) view_filter: ViewFilter,
    /// This System Monitor process's own uid — "me", for `ViewFilter::MyProcesses`
    /// / `ViewFilter::OtherUsersProcesses`. Recomputed each refresh (cheap: one
    /// lookup into the snapshot just taken); `None` only if the platform can't
    /// report it or our own pid is somehow absent from the snapshot.
    current_uid: Option<u32>,
}

/// A placeholder snapshot for before the first refresh, or if a projection
/// is ever rejected (e.g. a pathological duplicate-PID race) — the table
/// keeps rendering visible content either way; only the AT-SPI names would
/// be temporarily stale, an honest degradation rather than a crash.
fn empty_accessibility_snapshot() -> ProcessTableAccessibilitySnapshot {
    ProcessTableAccessibilitySnapshot {
        name: "Processes".to_string(),
        columns: Vec::new(),
        rows: Vec::new(),
        selected_row: None,
        row_count: 0,
        column_count: 0,
    }
}

impl ProcessTableDelegate {
    /// `visible` becomes the CPU tab's column set (the tab `MonitorView`
    /// always opens on) — typically the persisted or default columns from
    /// `columns::load`/`columns::default_visible`. Every other tab starts
    /// from its own [`columns::default_visible_for`].
    pub(crate) fn new(visible: Vec<ColKey>) -> Self {
        let mut tab_columns: [Vec<ColKey>; 5] = Default::default();
        for (index, tab) in Tab::ALL.into_iter().enumerate() {
            tab_columns[index] = if tab == Tab::Cpu {
                visible.clone()
            } else {
                columns::default_visible_for(tab)
            };
        }
        let mut delegate = Self {
            system: System::new(),
            all_rows: Vec::new(),
            rows: Vec::new(),
            columns: visible.iter().map(|key| key.to_column()).collect(),
            visible,
            users: Users::new_with_refreshed_list(),
            filter: String::new(),
            // Default: busiest CPU first.
            sort_key: ColKey::Cpu,
            sort_asc: false,
            selected_pid: None,
            accessible: empty_accessibility_snapshot(),
            refresh_count: 0,
            current_tab: Tab::Cpu,
            tab_columns,
            view_filter: ViewFilter::All,
            current_uid: None,
        };
        delegate.refresh();
        delegate
    }

    fn tab_index(tab: Tab) -> usize {
        Tab::ALL
            .iter()
            .position(|candidate| *candidate == tab)
            .unwrap_or(0)
    }

    /// Switch to `tab`'s own remembered column set (MON-02). A no-op if
    /// `tab` is already active.
    pub(crate) fn activate_tab(&mut self, tab: Tab) {
        if tab == self.current_tab {
            return;
        }
        self.current_tab = tab;
        self.visible = self.tab_columns[Self::tab_index(tab)].clone();
        self.columns = self.visible.iter().map(|key| key.to_column()).collect();
    }

    /// Set the selected PID and keep the AT-SPI projection's per-row
    /// `selected` flag in sync. This is the only place `selected_pid` should
    /// be assigned from outside this module.
    pub(crate) fn set_selected_pid(&mut self, pid: Option<u32>) {
        self.selected_pid = pid;
        self.refresh_accessible();
    }

    /// Recompute the bounded AT-SPI table projection from the current visible
    /// rows, columns, sort, and selection. Cheap and bounded (<=300 rows,
    /// <=11 columns) — called on data refresh, filter/sort/column changes,
    /// and selection changes, never from a per-frame render path.
    fn refresh_accessible(&mut self) {
        let columns: Vec<ProcessColumn> = self.visible.iter().map(|&key| key.into()).collect();
        let direction = if self.sort_asc {
            SortDirection::Ascending
        } else {
            SortDirection::Descending
        };
        self.accessible = accessibility::project_process_table(
            "Processes",
            &self.rows,
            &columns,
            self.selected_pid,
            self.sort_key.into(),
            direction,
        )
        .unwrap_or_else(|_| empty_accessibility_snapshot());
    }

    /// Show or hide a column. The Process Name anchor cannot be hidden, and the
    /// table never drops its last column.
    pub(crate) fn toggle_col(&mut self, key: ColKey) -> bool {
        if let Some(pos) = self.visible.iter().position(|&candidate| candidate == key) {
            if key.required() || self.visible.len() <= 1 {
                return false;
            }
            self.visible.remove(pos);
            if self.sort_key == key {
                self.sort_key = self.visible[0];
            }
        } else {
            let canonical = ColKey::ALL
                .iter()
                .position(|&candidate| candidate == key)
                .unwrap_or(0);
            let insert_at = self
                .visible
                .iter()
                .position(|&candidate| {
                    ColKey::ALL
                        .iter()
                        .position(|&entry| entry == candidate)
                        .unwrap_or(0)
                        > canonical
                })
                .unwrap_or(self.visible.len());
            self.visible.insert(insert_at, key);
        }
        self.columns = self.visible.iter().map(|key| key.to_column()).collect();
        self.tab_columns[Self::tab_index(self.current_tab)] = self.visible.clone();
        true
    }

    /// Change the View menu's process filter (MON-03) and reapply it.
    pub(crate) fn set_view_filter(&mut self, filter: ViewFilter) {
        self.view_filter = filter;
        self.apply_view();
    }

    pub(crate) fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.apply_view();
    }

    /// Pull a fresh snapshot from `sysinfo`, then reapply the active view.
    pub(crate) fn refresh(&mut self) {
        self.refresh_count = self.refresh_count.saturating_add(1);
        // The first refresh has no previous `sysinfo` reading to diff
        // against, so every process would otherwise read 0.0% CPU.
        let cpu_ready = self.refresh_count >= 2;

        // Frequency and static CPU metadata do not change on this screen; only
        // refresh usage deltas on the two-second sampling path.
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_disk_usage()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .with_user(UpdateKind::OnlyIfNotSet),
        );

        let users = &self.users;
        self.all_rows = self
            .system
            .processes()
            .values()
            // MON-01: sysinfo 0.33 lists every Linux thread under
            // `/proc/<pid>/task` as its own "process" too; keep only
            // thread-group leaders so Threads and Processes stop being the
            // same number and CPU/memory/disk totals stop double-counting.
            .filter(|process| is_thread_group_leader(process))
            .map(|process| {
                let disk_usage = process.disk_usage();
                let mut cmd_search = String::new();
                if let Some(executable) = process.exe() {
                    cmd_search.push_str(&executable.to_string_lossy());
                }
                for argument in process.cmd() {
                    cmd_search.push(' ');
                    cmd_search.push_str(&argument.to_string_lossy());
                }
                cmd_search.make_ascii_lowercase();

                let cpu = process.cpu_usage();
                let disk = disk_usage.read_bytes + disk_usage.written_bytes;
                let uid = process.user_id().map(|uid| **uid);
                let user = process
                    .user_id()
                    .and_then(|uid| users.get_user_by_id(uid))
                    .map(|user| SharedString::from(user.name().to_string()))
                    .or_else(|| uid.map(|uid| SharedString::from(format!("uid {uid}"))))
                    .unwrap_or_else(|| SharedString::from("—"));
                ProcRow {
                    pid: process.pid().as_u32(),
                    name: process.name().to_string_lossy().into_owned().into(),
                    cmd_search: cmd_search.into(),
                    cpu,
                    cpu_ready,
                    mem: process.memory(),
                    disk,
                    energy: cpu + (disk as f32 / 1_048_576.0) * 0.5,
                    ppid: process.parent().map(|parent| parent.as_u32()),
                    user,
                    vmem: process.virtual_memory(),
                    run_time: process.run_time(),
                    start_time: process.start_time(),
                    status: SharedString::from(process.status().to_string()),
                    threads: thread_group_size(process.tasks().map(|tasks| tasks.len())),
                    uid,
                }
            })
            .collect();

        // MON-03: "me", for the View filter's My/Other Users' Processes.
        self.current_uid = sysinfo::get_current_pid()
            .ok()
            .and_then(|pid| self.system.process(pid))
            .and_then(|process| process.user_id())
            .map(|uid| **uid);

        self.apply_view();
    }

    /// Rebuild the bounded visible rows from the complete snapshot.
    pub(crate) fn apply_view(&mut self) {
        let needle = self.filter.to_lowercase();
        let view_filter = self.view_filter;
        let current_uid = self.current_uid;
        Self::sort_rows(&mut self.all_rows, self.sort_key, self.sort_asc);
        self.rows = self
            .all_rows
            .iter()
            .filter(|row| {
                (needle.is_empty()
                    || row.name.to_lowercase().contains(&needle)
                    || row.pid.to_string().contains(&needle)
                    || row.cmd_search.contains(&needle))
                    && view_filter.matches(row.uid, row.status.as_ref(), current_uid)
            })
            .take(300)
            .cloned()
            .collect();
        self.refresh_accessible();
    }

    fn sort_rows(rows: &mut [ProcRow], key: ColKey, ascending: bool) {
        rows.sort_by(|left, right| {
            let ordering = match key {
                ColKey::Pid => left.pid.cmp(&right.pid),
                ColKey::Name => left.name.cmp(&right.name),
                ColKey::Cpu => left.cpu.partial_cmp(&right.cpu).unwrap_or(Ordering::Equal),
                ColKey::Mem => left.mem.cmp(&right.mem),
                ColKey::Energy => left
                    .energy
                    .partial_cmp(&right.energy)
                    .unwrap_or(Ordering::Equal),
                ColKey::Disk => left.disk.cmp(&right.disk),
                ColKey::Ppid => left.ppid.cmp(&right.ppid),
                ColKey::User => left.user.cmp(&right.user),
                ColKey::Vmem => left.vmem.cmp(&right.vmem),
                ColKey::RunTime => left.run_time.cmp(&right.run_time),
                ColKey::Status => left.status.cmp(&right.status),
                ColKey::Threads => left.threads.cmp(&right.threads),
            };
            if ascending {
                ordering
            } else {
                ordering.reverse()
            }
        });
    }
}

/// The `ColumnSort` a header's Space/AT-SPI-Click activation should apply:
/// flip direction on the column already sorted, default a newly-chosen one
/// to ascending. Pure model behind [`ProcessTableDelegate::activate_sort`].
fn next_header_sort(already_sorting: bool, currently_ascending: bool) -> ColumnSort {
    if already_sorting && currently_ascending {
        ColumnSort::Descending
    } else {
        ColumnSort::Ascending
    }
}

/// Select the row at `row_index` exactly the way a mouse click does — used by
/// this table's own mouse handlers and, identically, by the AT-SPI Click and
/// Focus actions wired on each row in `render_tr`.
fn select_row(
    state: &mut TableState<ProcessTableDelegate>,
    row_index: usize,
    cx: &mut Context<TableState<ProcessTableDelegate>>,
) {
    let pid = state.delegate().rows.get(row_index).map(|row| row.pid);
    state.delegate_mut().set_selected_pid(pid);
    state.set_selected_row(row_index, cx);
}

/// Re-point the selected row at the stored PID after refresh/filter/sort. A PID
/// hidden by the current filter remains retained; a vanished PID is forgotten.
pub(crate) fn resync_selection(
    state: &mut TableState<ProcessTableDelegate>,
    cx: &mut Context<TableState<ProcessTableDelegate>>,
) {
    let (visible_index, retained_pid) = {
        let delegate = state.delegate();
        selection_projection(
            delegate.selected_pid,
            delegate.rows.iter().map(|row| row.pid),
            delegate.all_rows.iter().map(|row| row.pid),
        )
    };
    state.delegate_mut().set_selected_pid(retained_pid);
    match visible_index {
        Some(index) => state.set_selected_row(index, cx),
        None if state.selected_row().is_some() => state.clear_selection(cx),
        None => {}
    }
}

fn selection_projection(
    selected_pid: Option<u32>,
    visible: impl IntoIterator<Item = u32>,
    all: impl IntoIterator<Item = u32>,
) -> (Option<usize>, Option<u32>) {
    let Some(pid) = selected_pid else {
        return (None, None);
    };
    let visible_index = visible.into_iter().position(|candidate| candidate == pid);
    let retained_pid = all
        .into_iter()
        .any(|candidate| candidate == pid)
        .then_some(pid);
    (visible_index, retained_pid)
}

impl TableDelegate for ProcessTableDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, column_index: usize, _cx: &App) -> Column {
        self.columns[column_index].clone()
    }

    fn perform_sort(
        &mut self,
        column_index: usize,
        sort: ColumnSort,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        if let Some(&key) = self.visible.get(column_index) {
            self.sort_key = key;
        }
        self.sort_asc = matches!(sort, ColumnSort::Ascending);
        self.apply_view();
        cx.defer_in(window, |state, _window, cx| resync_selection(state, cx));
    }

    fn render_header(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> Stateful<gpui::Div> {
        div().id("header").role(Role::Row)
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let name = self.columns[col_ix].name.clone();
        // `self.accessible.columns` is rebuilt alongside `self.rows` on every
        // sort (`apply_view` -> `refresh_accessible`), so this always
        // reflects the real, current sort order — including one set from
        // the keyboard/AT-SPI path below, which never touches the table
        // widget's own (private, mouse-only) sort-icon state.
        let sort = self
            .accessible
            .columns
            .get(col_ix)
            .and_then(|column| column.sort);
        let accessible_name: SharedString = match sort {
            Some(SortDirection::Ascending) => format!("{name}, sorted ascending").into(),
            Some(SortDirection::Descending) => format!("{name}, sorted descending").into(),
            None => name.clone(),
        };
        let focus = window
            .use_keyed_state(("col-header-focus", col_ix), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let focused = focus.is_focused(window);
        let view = cx.entity();
        div()
            .id(("col-header-name", col_ix))
            .role(Role::ColumnHeader)
            .aria_label(accessible_name)
            .size_full()
            .cursor_pointer()
            .track_focus(&focus.tab_stop(true).tab_index(0))
            .when(focused, |el| el.shadow(rmac_ui::mac::focus_ring_shadow()))
            // Deliberately not `on_click`: the table widget's own header
            // wrapper already calls back into `perform_sort` on a real mouse
            // click (bubbled up from this div), and GPUI's generic
            // Space/Return-activates-a-focused-click mapping only fires a
            // div's own listeners, not its ancestors' — adding a second
            // `on_click` here would double-fire (and double-toggle the sort
            // direction) on every mouse click. Space is handled directly
            // instead, entirely independent of the mouse path.
            .on_key_down({
                let view = view.clone();
                move |event: &gpui::KeyDownEvent, window, cx| {
                    if event.keystroke.key.as_str() == "space"
                        && !event.keystroke.modifiers.modified()
                    {
                        cx.stop_propagation();
                        view.update(cx, |table, cx| {
                            table.delegate_mut().activate_sort(col_ix, window, cx);
                        });
                    }
                }
            })
            .on_a11y_action(AccessibleAction::Click, move |_data, window, cx| {
                view.update(cx, |table, cx| {
                    table.delegate_mut().activate_sort(col_ix, window, cx);
                });
            })
            .child(name)
    }

    fn render_tr(
        &mut self,
        row_index: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<gpui::Div> {
        let selected = self
            .accessible
            .rows
            .get(row_index)
            .map(|row| row.selected)
            .unwrap_or(false);
        let name = self
            .accessible
            .rows
            .get(row_index)
            .map(|row| row.label.clone())
            .or_else(|| self.rows.get(row_index).map(|row| row.name.to_string()))
            .unwrap_or_default();
        let description = self
            .rows
            .get(row_index)
            .map(|row| format!("{:.1}% CPU, {}", row.cpu, format_mem(row.mem)))
            .unwrap_or_default();
        let accessible_name = if description.is_empty() {
            name
        } else {
            format!("{name}, {description}")
        };
        let view = cx.entity();
        div()
            .id(("row", row_index))
            .role(Role::Row)
            .aria_label(SharedString::from(accessible_name))
            .aria_selected(selected)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |state, _, _, cx| select_row(state, row_index, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |state, _, _, cx| select_row(state, row_index, cx)),
            )
            // A screen reader selects a row exactly like a mouse click: both
            // paths funnel through `set_selected_pid` + `set_selected_row`.
            .on_a11y_action(AccessibleAction::Click, {
                let view = view.clone();
                move |_data, _window, cx| {
                    view.update(cx, |state, cx| select_row(state, row_index, cx));
                }
            })
            .on_a11y_action(AccessibleAction::Focus, move |_data, _window, cx| {
                view.update(cx, |state, cx| select_row(state, row_index, cx));
            })
    }

    fn context_menu(
        &mut self,
        row_index: usize,
        menu: PopupMenu,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        // This stays on the table widget because its right-clicked row index is
        // not otherwise exposed; replacing it would duplicate table hit-testing.
        let pid = self.rows.get(row_index).map(|row| row.pid);
        if pid.is_some() {
            self.set_selected_pid(pid);
        }
        menu.menu("Quit", Box::new(QuitProcess))
            .menu("Force Quit", Box::new(ForceQuitProcess))
    }

    fn render_td(
        &mut self,
        row_index: usize,
        column_index: usize,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = &self.rows[row_index];
        let key = self
            .visible
            .get(column_index)
            .copied()
            .unwrap_or(ColKey::Name);
        let text: SharedString = row.cell_text(key).into();
        div().child(text)
    }
}

impl ProcessTableDelegate {
    /// Sort by `column_index` from the keyboard (Space on a focused header)
    /// or AT-SPI (`AccessibleAction::Click`), reusing the exact same
    /// [`Self::perform_sort`] the table's own mouse-driven header click
    /// calls: re-activating the current sort column flips its direction,
    /// picking a new one defaults it to ascending. This updates the real
    /// sort order and `self.accessible`'s per-column `sort` field, which is
    /// what `render_th`'s own `aria_label` reads — but not the table
    /// widget's private sort-icon state, which only its own mouse-click path
    /// (already unaffected by this) can reach; an honest limitation, not
    /// simulated behaviour.
    pub(crate) fn activate_sort(
        &mut self,
        column_index: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        let already_sorting = self.visible.get(column_index) == Some(&self.sort_key);
        let sort = next_header_sort(already_sorting, self.sort_asc);
        self.perform_sort(column_index, sort, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_leader_thread_kind, next_header_sort, selection_projection, thread_group_size, ColKey,
        ProcRow,
    };
    use rmac_ui::ColumnSort;
    use sysinfo::ThreadKind;

    #[test]
    fn header_sort_flips_direction_on_the_active_column() {
        assert_eq!(next_header_sort(true, true), ColumnSort::Descending);
        assert_eq!(next_header_sort(true, false), ColumnSort::Ascending);
    }

    #[test]
    fn header_sort_defaults_a_new_column_to_ascending() {
        assert_eq!(next_header_sort(false, true), ColumnSort::Ascending);
        assert_eq!(next_header_sort(false, false), ColumnSort::Ascending);
    }

    #[test]
    fn process_churn_clears_only_a_vanished_selection() {
        assert_eq!(
            selection_projection(Some(20), [10, 20, 30], [10, 20, 30]),
            (Some(1), Some(20))
        );
        assert_eq!(
            selection_projection(Some(20), [10, 30], [10, 20, 30]),
            (None, Some(20))
        );
        assert_eq!(
            selection_projection(Some(20), [10, 30], [10, 30]),
            (None, None)
        );
    }

    fn row(cpu: f32, cpu_ready: bool) -> ProcRow {
        ProcRow {
            pid: 1,
            name: "proc".into(),
            cmd_search: "proc".into(),
            cpu,
            cpu_ready,
            mem: 0,
            disk: 0,
            energy: 0.0,
            ppid: None,
            user: "user".into(),
            vmem: 0,
            run_time: 0,
            start_time: 0,
            status: "Running".into(),
            threads: 1,
            uid: None,
        }
    }

    #[test]
    fn cpu_reads_a_dash_until_the_second_sample() {
        assert_eq!(row(0.0, false).cell_text(ColKey::Cpu), "—");
        // Even a nonzero first-sample reading (sysinfo's own transient
        // values) stays hidden until a real delta exists.
        assert_eq!(row(12.5, false).cell_text(ColKey::Cpu), "—");
        assert_eq!(row(0.0, true).cell_text(ColKey::Cpu), "0.0");
        assert_eq!(row(12.5, true).cell_text(ColKey::Cpu), "12.5");
    }

    #[test]
    fn threads_column_shows_the_thread_group_size() {
        let mut leader = row(1.0, true);
        leader.threads = 7;
        assert_eq!(leader.cell_text(ColKey::Threads), "7");
    }

    /// MON-01 fixture: a small table standing in for what `sysinfo` 0.33
    /// hands back per `Process::thread_kind()` on Linux — `None` for a real
    /// thread-group leader (`pid == tgid`, one row per running program, the
    /// Mac's notion of "process"), `Some(Userland)` for every *other* thread
    /// `sysinfo` also finds under `/proc/<pid>/task`, and `Some(Kernel)` for
    /// a kernel thread such as `[kworker/0:0]` (its own independent
    /// scheduling unit with `pid == tgid`, kept the same way `ps`/`top` keep
    /// it). Before this fix, `process_table` counted all three kinds as
    /// "processes", so Threads == Processes and every sum double-counted.
    #[test]
    fn only_userland_thread_entries_are_filtered_out() {
        let fixture = [
            (None, true, "a real process (thread-group leader)"),
            (
                Some(ThreadKind::Userland),
                false,
                "another thread of that same process",
            ),
            (
                Some(ThreadKind::Kernel),
                true,
                "a kernel thread, its own independent process",
            ),
        ];
        for (thread_kind, expected_leader, description) in fixture {
            assert_eq!(
                is_leader_thread_kind(thread_kind),
                expected_leader,
                "{description}: thread_kind={thread_kind:?}"
            );
        }
    }

    #[test]
    fn thread_group_size_is_leader_plus_other_tasks() {
        // `Process::tasks()` is `None` on a platform sysinfo can't walk
        // `/proc/<pid>/task` on, or when a process happens to have no other
        // threads at all — either way, the leader itself still counts.
        assert_eq!(thread_group_size(None), 1);
        assert_eq!(thread_group_size(Some(0)), 1);
        assert_eq!(thread_group_size(Some(6)), 7);
    }
}
