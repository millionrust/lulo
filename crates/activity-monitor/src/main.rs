//! rmac Activity Monitor — a fast, native, GPU-rendered process monitor.
//!
//! Phase 1 of the rmac desktop suite. Built on GPUI + gpui-component, with
//! `sysinfo` as the data layer. Runs on macOS today (Metal) and targets
//! Ubuntu/Wayland (Vulkan) later — the same binary, no webview, instant launch.

mod cpu_ticks;

use std::cmp::Ordering;
use std::time::Duration;

use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, Context, Entity,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement, Render, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonGroup, ButtonVariants as _},
    input::{Input, InputState},
    menu::PopupMenu,
    table::{Column, ColumnSort, Table, TableDelegate, TableEvent, TableState},
    Disableable as _, Selectable as _, Sizable as _, StyledExt as _,
};
use rmac_ui::mac;
use sysinfo::{Networks, Pid, ProcessesToUpdate, Signal, System, Users};

gpui::actions!(
    activity_monitor,
    [QuitProcess, ForceQuitProcess, FocusSearch, CancelKill, ConfirmKill]
);

/// Which top-level pane is active. Each tab re-focuses the table on a different
/// metric (default sort) and surfaces a different aggregate summary + sparkline.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Cpu,
    Memory,
    Energy,
    Disk,
    Network,
}

impl Tab {
    const ALL: [Tab; 5] = [Tab::Cpu, Tab::Memory, Tab::Energy, Tab::Disk, Tab::Network];

    fn label(self) -> &'static str {
        match self {
            Tab::Cpu => "CPU",
            Tab::Memory => "Memory",
            Tab::Energy => "Energy",
            Tab::Disk => "Disk",
            Tab::Network => "Network",
        }
    }

    /// The column the table sorts by (descending) when this tab is activated.
    ///
    /// Network has no per-process data source (sysinfo only exposes system-wide
    /// interface counters), so it is summary-only and hides the table — the
    /// returned column is unused there but kept sensible for safety.
    fn default_sort_key(self) -> ColKey {
        match self {
            Tab::Cpu => ColKey::Cpu,
            Tab::Memory => ColKey::Mem,
            Tab::Energy => ColKey::Energy,
            Tab::Disk => ColKey::Disk,
            Tab::Network => ColKey::Cpu,
        }
    }

    /// Whether this tab shows the per-process table. Network is summary-only
    /// because there is no reliable per-process network data on macOS/Linux here.
    fn has_process_table(self) -> bool {
        !matches!(self, Tab::Network)
    }
}

/// A column in the process table. The set is fixed and canonically ordered; the
/// column chooser toggles which ones are visible. Only columns backed by real
/// `sysinfo` data exist here — no placeholder/fabricated metrics.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ColKey {
    Pid,
    Name,
    Cpu,
    Mem,
    Energy,
    Disk,
    Ppid,
    User,
    Vmem,
    RunTime,
    Status,
}

impl ColKey {
    /// Canonical order, also the order shown in the chooser.
    const ALL: [ColKey; 11] = [
        ColKey::Pid,
        ColKey::Name,
        ColKey::Cpu,
        ColKey::Mem,
        ColKey::Energy,
        ColKey::Disk,
        ColKey::Ppid,
        ColKey::User,
        ColKey::Vmem,
        ColKey::RunTime,
        ColKey::Status,
    ];

    fn id(self) -> &'static str {
        match self {
            ColKey::Pid => "pid",
            ColKey::Name => "name",
            ColKey::Cpu => "cpu",
            ColKey::Mem => "mem",
            ColKey::Energy => "energy",
            ColKey::Disk => "disk",
            ColKey::Ppid => "ppid",
            ColKey::User => "user",
            ColKey::Vmem => "vmem",
            ColKey::RunTime => "runtime",
            ColKey::Status => "status",
        }
    }

    fn from_id(s: &str) -> Option<ColKey> {
        ColKey::ALL.into_iter().find(|k| k.id() == s)
    }

    fn title(self) -> &'static str {
        match self {
            ColKey::Pid => "PID",
            ColKey::Name => "Process Name",
            ColKey::Cpu => "% CPU",
            ColKey::Mem => "Memory",
            ColKey::Energy => "Energy",
            ColKey::Disk => "Disk I/O",
            ColKey::Ppid => "Parent PID",
            ColKey::User => "User",
            ColKey::Vmem => "Virtual Mem",
            ColKey::RunTime => "Run Time",
            ColKey::Status => "Status",
        }
    }

    fn width(self) -> f32 {
        match self {
            ColKey::Pid => 72.0,
            ColKey::Name => 280.0,
            ColKey::Cpu => 96.0,
            ColKey::Mem => 110.0,
            ColKey::Energy => 96.0,
            ColKey::Disk => 120.0,
            ColKey::Ppid => 96.0,
            ColKey::User => 130.0,
            ColKey::Vmem => 120.0,
            ColKey::RunTime => 110.0,
            ColKey::Status => 110.0,
        }
    }

    fn right(self) -> bool {
        matches!(
            self,
            ColKey::Pid
                | ColKey::Cpu
                | ColKey::Mem
                | ColKey::Energy
                | ColKey::Disk
                | ColKey::Ppid
                | ColKey::Vmem
                | ColKey::RunTime
        )
    }

    fn default_visible(self) -> bool {
        matches!(
            self,
            ColKey::Pid | ColKey::Name | ColKey::Cpu | ColKey::Mem | ColKey::Energy | ColKey::Disk
        )
    }

    /// The Process Name column is the human anchor — never hide it.
    fn required(self) -> bool {
        matches!(self, ColKey::Name)
    }

    fn to_column(self) -> Column {
        let c = Column::new(self.id(), self.title()).width(px(self.width())).sortable();
        if self.right() {
            c.text_right()
        } else {
            c
        }
    }
}

/// A pending Quit / Force Quit awaiting user confirmation.
#[derive(Clone)]
struct PendingKill {
    pid: u32,
    name: SharedString,
    force: bool,
}

/// One row in the process table — a flat snapshot, cheap to clone/diff.
#[derive(Clone)]
struct ProcRow {
    pid: u32,
    name: SharedString,
    /// Lower-cased command line / executable path, used for search matching only.
    cmd_search: String,
    cpu: f32,
    mem: u64,
    /// Bytes read+written since the last refresh (a per-tick I/O proxy).
    disk: u64,
    /// Energy-impact proxy. sysinfo has no real "energy impact"; CPU usage is
    /// the dominant term in macOS's own figure, so we approximate with it.
    energy: f32,
    /// Parent process id (real, from sysinfo).
    ppid: Option<u32>,
    /// Owning user name, resolved from the process uid.
    user: SharedString,
    /// Virtual memory size in bytes.
    vmem: u64,
    /// Wall-clock run time in seconds since the process started.
    run_time: u64,
    /// Process status (Running / Sleeping / …), for sorting + display.
    status: SharedString,
}

/// Table delegate: owns the live `System` handle plus the current snapshot.
///
/// We keep a single `System` instance and refresh it in place so `sysinfo`
/// can compute per-process CPU deltas between ticks (it works on diffs).
struct ProcessTableDelegate {
    system: System,
    /// Full unfiltered snapshot.
    all_rows: Vec<ProcRow>,
    /// Filtered + sorted rows actually shown.
    rows: Vec<ProcRow>,
    /// The visible columns, in display order — a subset of `ColKey::ALL`.
    visible: Vec<ColKey>,
    /// `gpui-component` column descriptors mirroring `visible`.
    columns: Vec<Column>,
    /// uid → username resolution table, refreshed alongside the process list.
    users: Users,
    cpu_count: usize,
    filter: String,
    /// The column the rows are sorted by — identity-based so it survives the
    /// visible set changing under it.
    sort_key: ColKey,
    sort_asc: bool,
    /// The PID the user has selected. This is the source of truth for the
    /// selection — the table re-sorts every tick, so a stored row index would
    /// drift onto a different process. The row index is derived from this for
    /// rendering (and re-synced after every refresh/filter/sort).
    selected_pid: Option<u32>,
}

impl ProcessTableDelegate {
    fn new() -> Self {
        let visible = load_visible_cols();
        let mut delegate = Self {
            system: System::new_all(),
            all_rows: Vec::new(),
            rows: Vec::new(),
            columns: visible.iter().map(|k| k.to_column()).collect(),
            visible,
            users: Users::new_with_refreshed_list(),
            cpu_count: 1,
            filter: String::new(),
            // Default: busiest CPU first.
            sort_key: ColKey::Cpu,
            sort_asc: false,
            selected_pid: None,
        };
        delegate.refresh();
        delegate
    }

    /// Rebuild the `gpui-component` column list from the visible set.
    fn rebuild_columns(&mut self) {
        self.columns = self.visible.iter().map(|k| k.to_column()).collect();
    }

    /// Show or hide a column. The Process Name anchor can't be hidden, and we
    /// never drop the last column. Returns whether anything changed.
    fn toggle_col(&mut self, key: ColKey) -> bool {
        if let Some(pos) = self.visible.iter().position(|&k| k == key) {
            if key.required() || self.visible.len() <= 1 {
                return false;
            }
            self.visible.remove(pos);
            // If we hid the sort column, fall back to the first visible one.
            if self.sort_key == key {
                self.sort_key = self.visible[0];
            }
        } else {
            // Re-insert in canonical order.
            let canon = ColKey::ALL.iter().position(|&k| k == key).unwrap_or(0);
            let insert_at = self
                .visible
                .iter()
                .position(|&k| ColKey::ALL.iter().position(|&c| c == k).unwrap_or(0) > canon)
                .unwrap_or(self.visible.len());
            self.visible.insert(insert_at, key);
        }
        self.rebuild_columns();
        save_visible_cols(&self.visible);
        true
    }

    /// Pull a fresh snapshot from `sysinfo`, then re-apply the active filter/sort.
    fn refresh(&mut self) {
        self.system.refresh_cpu_all();
        self.system.refresh_memory();
        self.system
            .refresh_processes(ProcessesToUpdate::All, true);
        self.cpu_count = self.system.cpus().len().max(1);
        self.users.refresh();

        let users = &self.users;
        self.all_rows = self
            .system
            .processes()
            .values()
            .map(|p| {
                let du = p.disk_usage();
                // Build a searchable haystack from the exe path + full command line
                // so search can match on path/command, not just the process name.
                let mut cmd_search = String::new();
                if let Some(exe) = p.exe() {
                    cmd_search.push_str(&exe.to_string_lossy());
                }
                for arg in p.cmd() {
                    cmd_search.push(' ');
                    cmd_search.push_str(&arg.to_string_lossy());
                }
                cmd_search.make_ascii_lowercase();
                let user = p
                    .user_id()
                    .and_then(|uid| users.get_user_by_id(uid))
                    .map(|u| SharedString::from(u.name().to_string()))
                    .or_else(|| p.user_id().map(|uid| SharedString::from(format!("uid {}", **uid))))
                    .unwrap_or_else(|| SharedString::from("—"));
                ProcRow {
                    pid: p.pid().as_u32(),
                    name: p.name().to_string_lossy().into_owned().into(),
                    cmd_search,
                    cpu: p.cpu_usage(),
                    mem: p.memory(),
                    disk: du.read_bytes + du.written_bytes,
                    energy: p.cpu_usage(),
                    ppid: p.parent().map(|pp| pp.as_u32()),
                    user,
                    vmem: p.virtual_memory(),
                    run_time: p.run_time(),
                    status: SharedString::from(p.status().to_string()),
                }
            })
            .collect();

        self.apply_view();
    }

    /// Rebuild `rows` from `all_rows` using the current filter and sort.
    fn apply_view(&mut self) {
        let needle = self.filter.to_lowercase();
        let mut rows: Vec<ProcRow> = if needle.is_empty() {
            self.all_rows.clone()
        } else {
            self.all_rows
                .iter()
                .filter(|r| {
                    r.name.to_lowercase().contains(&needle)
                        || r.pid.to_string().contains(&needle)
                        || r.cmd_search.contains(&needle)
                })
                .cloned()
                .collect()
        };
        Self::sort_rows(&mut rows, self.sort_key, self.sort_asc);
        rows.truncate(300);
        self.rows = rows;
    }

    fn sort_rows(rows: &mut [ProcRow], key: ColKey, asc: bool) {
        rows.sort_by(|a, b| {
            let o = match key {
                ColKey::Pid => a.pid.cmp(&b.pid),
                ColKey::Name => a.name.cmp(&b.name),
                ColKey::Cpu => a.cpu.partial_cmp(&b.cpu).unwrap_or(Ordering::Equal),
                ColKey::Mem => a.mem.cmp(&b.mem),
                ColKey::Energy => a.energy.partial_cmp(&b.energy).unwrap_or(Ordering::Equal),
                ColKey::Disk => a.disk.cmp(&b.disk),
                ColKey::Ppid => a.ppid.cmp(&b.ppid),
                ColKey::User => a.user.cmp(&b.user),
                ColKey::Vmem => a.vmem.cmp(&b.vmem),
                ColKey::RunTime => a.run_time.cmp(&b.run_time),
                ColKey::Status => a.status.cmp(&b.status),
            };
            if asc {
                o
            } else {
                o.reverse()
            }
        });
    }
}

/// Re-point the table's selected row at the stored PID after the rows have been
/// rebuilt/re-sorted/filtered. If the PID has vanished entirely, forget it; if
/// it is merely hidden by the current filter, keep the PID but clear the visible
/// highlight so it returns when the filter is cleared.
fn resync_selection(
    state: &mut TableState<ProcessTableDelegate>,
    cx: &mut Context<TableState<ProcessTableDelegate>>,
) {
    let (visible_ix, gone) = {
        let d = state.delegate();
        match d.selected_pid {
            Some(pid) => (
                d.rows.iter().position(|r| r.pid == pid),
                !d.all_rows.iter().any(|r| r.pid == pid),
            ),
            None => (None, false),
        }
    };
    if gone {
        state.delegate_mut().selected_pid = None;
    }
    match visible_ix {
        Some(ix) => state.set_selected_row(ix, cx),
        None => {
            if state.selected_row().is_some() {
                state.clear_selection(cx);
            }
        }
    }
}

fn format_mem(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else {
        format!("{:.0} KB", b / KB)
    }
}

/// Format an elapsed-seconds duration as `H:MM:SS` (or `M:SS` under an hour).
fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Path to the persisted visible-columns file.
fn cols_config_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let dir =
        std::path::Path::new(&home).join("Library/Application Support/rmac-activity-monitor");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("columns.txt"))
}

/// Load the visible column set (comma-separated ids), falling back to defaults.
fn load_visible_cols() -> Vec<ColKey> {
    let parsed: Option<Vec<ColKey>> = cols_config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.split(',').filter_map(|t| ColKey::from_id(t.trim())).collect());
    let cols = parsed.filter(|v: &Vec<ColKey>| !v.is_empty()).unwrap_or_else(|| {
        ColKey::ALL.into_iter().filter(|k| k.default_visible()).collect()
    });
    // Process Name is the anchor — guarantee it's present.
    if cols.contains(&ColKey::Name) {
        cols
    } else {
        let mut c = cols;
        c.insert(0, ColKey::Name);
        c
    }
}

fn save_visible_cols(cols: &[ColKey]) {
    if let Some(p) = cols_config_path() {
        let line = cols.iter().map(|k| k.id()).collect::<Vec<_>>().join(",");
        let _ = std::fs::write(p, line);
    }
}

/// Format a byte-rate (bytes per second) compactly.
fn format_rate(bytes_per_s: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    if bytes_per_s >= GB {
        format!("{:.2} GB/s", bytes_per_s / GB)
    } else if bytes_per_s >= MB {
        format!("{:.1} MB/s", bytes_per_s / MB)
    } else {
        format!("{:.0} KB/s", bytes_per_s / KB)
    }
}

/// Format a cumulative byte count compactly (e.g. "3.42 GB").
fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

impl TableDelegate for ProcessTableDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> &Column {
        &self.columns[col_ix]
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        if let Some(&key) = self.visible.get(col_ix) {
            self.sort_key = key;
        }
        self.sort_asc = matches!(sort, ColumnSort::Ascending);
        self.apply_view();
        // Rows just re-sorted, so the stored row index is stale — re-point the
        // highlight at the selected PID once the table is back on the stack.
        cx.defer_in(window, |state, _window, cx| resync_selection(state, cx));
    }

    /// Right-clicking a row should also select it, so the context menu and the
    /// toolbar/keyboard actions all act on the same target.
    fn render_tr(
        &mut self,
        row_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<gpui::Div> {
        div()
            .id(("row", row_ix))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |state, _, _, cx| {
                    let pid = state.delegate().rows.get(row_ix).map(|r| r.pid);
                    state.delegate_mut().selected_pid = pid;
                    state.set_selected_row(row_ix, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |state, _, _, cx| {
                    let pid = state.delegate().rows.get(row_ix).map(|r| r.pid);
                    state.delegate_mut().selected_pid = pid;
                    state.set_selected_row(row_ix, cx);
                }),
            )
    }

    fn context_menu(
        &mut self,
        _row_ix: usize,
        menu: PopupMenu,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        menu.menu("Quit", Box::new(QuitProcess))
            .menu("Force Quit", Box::new(ForceQuitProcess))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = &self.rows[row_ix];
        let key = self.visible.get(col_ix).copied().unwrap_or(ColKey::Name);
        let text: SharedString = match key {
            ColKey::Pid => row.pid.to_string().into(),
            ColKey::Name => row.name.clone(),
            ColKey::Cpu => format!("{:.1}", row.cpu).into(),
            ColKey::Mem => format_mem(row.mem).into(),
            ColKey::Energy => format!("{:.1}", row.energy).into(),
            ColKey::Disk => format_mem(row.disk).into(),
            ColKey::Ppid => row.ppid.map(|p| p.to_string()).unwrap_or_default().into(),
            ColKey::User => row.user.clone(),
            ColKey::Vmem => format_mem(row.vmem).into(),
            ColKey::RunTime => format_duration(row.run_time).into(),
            ColKey::Status => row.status.clone(),
        };
        div().child(text)
    }
}

/// Aggregate readings recomputed every tick, used by the summary strip + graphs.
#[derive(Default)]
struct Aggregates {
    cpu_total: f32,
    per_core: Vec<f32>,
    mem_used: u64,
    mem_total: u64,
    mem_available: u64,
    swap_used: u64,
    swap_total: u64,
    energy_total: f32,
    disk_read_rate: f64,
    disk_write_rate: f64,
    net_recv_rate: f64,
    net_sent_rate: f64,
}

/// A short ring of recent samples for each metric, drawn as a bar sparkline.
#[derive(Default)]
struct History {
    cpu: Vec<f32>,
    mem: Vec<f32>,
    energy: Vec<f32>,
    disk: Vec<f32>,
    net: Vec<f32>,
}

impl History {
    const CAP: usize = 60;

    fn push(buf: &mut Vec<f32>, v: f32) {
        buf.push(v);
        if buf.len() > Self::CAP {
            buf.remove(0);
        }
    }
}

const REFRESH_SECS: f64 = 2.0;

/// Root view: a tabbed summary + the live process table.
struct MonitorView {
    table: Entity<TableState<ProcessTableDelegate>>,
    search: Entity<InputState>,
    focus: gpui::FocusHandle,
    networks: Networks,
    tab: Tab,
    agg: Aggregates,
    history: History,
    pending_kill: Option<PendingKill>,
    /// Whether the column chooser dropdown is open.
    cols_menu_open: bool,
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

/// One row in the Network tab's per-interface table.
#[derive(Clone)]
struct NetIface {
    name: String,
    total_recv: u64,
    total_sent: u64,
    recv_rate: f64,
    sent_rate: f64,
}

impl MonitorView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| TableState::new(ProcessTableDelegate::new(), window, cx));
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
            cols_menu_open: false,
            inspect_pid: None,
            net_ifaces: Vec::new(),
            prev_cpu_ticks: None,
            cpu_split: None,
        };
        view.refresh(cx);

        // Auto-refresh loop — every 2s, off the render path.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(2))
                    .await;
                let Some(this) = this.upgrade() else { break };
                let updated = cx.update_entity(&this, |view: &mut MonitorView, cx| {
                    view.refresh(cx);
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();

        view
    }

    fn apply_filter(&mut self, cx: &mut Context<Self>) {
        let q = self.search.read(cx).value().to_string();
        self.table.update(cx, |state, cx| {
            state.delegate_mut().filter = q;
            state.delegate_mut().apply_view();
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

            let disk_bytes: u64 = delegate.all_rows.iter().map(|r| r.disk).sum();
            // Split is approximate; we only have the combined per-row figure here,
            // so re-derive read/write from the live processes.
            let (read, write) = delegate.system.processes().values().fold(
                (0u64, 0u64),
                |(r, w), p| {
                    let du = p.disk_usage();
                    (r + du.read_bytes, w + du.written_bytes)
                },
            );
            let _ = disk_bytes;
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

    fn selected_proc(&self, cx: &Context<Self>) -> Option<(u32, SharedString)> {
        let state = self.table.read(cx);
        let d = state.delegate();
        let pid = d.selected_pid?;
        let name = d
            .rows
            .iter()
            .find(|r| r.pid == pid)
            .or_else(|| d.all_rows.iter().find(|r| r.pid == pid))
            .map(|r| r.name.clone())
            .unwrap_or_else(|| SharedString::from(format!("PID {pid}")));
        Some((pid, name))
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
        if let Some((pid, name)) = self.selected_proc(cx) {
            self.pending_kill = Some(PendingKill { pid, name, force });
            cx.notify();
        }
    }

    fn confirm_kill(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.pending_kill.take() {
            self.table.update(cx, |state, _| {
                if let Some(proc) = state.delegate().system.process(Pid::from_u32(p.pid)) {
                    let signal = if p.force { Signal::Kill } else { Signal::Term };
                    let _ = proc.kill_with(signal);
                }
            });
            self.refresh(cx);
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
        self.table.update(cx, |state, cx| {
            if state.delegate_mut().toggle_col(key) {
                state.delegate_mut().apply_view();
                resync_selection(state, cx);
                state.refresh(cx);
            }
        });
        cx.notify();
    }

    /// The column-chooser dropdown: a checklist of every available column.
    fn render_columns_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let visible = self.table.read(cx).delegate().visible.clone();
        div()
            .absolute()
            .top(px(96.0))
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
                    .text_size(px(11.0))
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
                    .text_size(px(12.0))
                    .text_color(if disabled { mac::text_tertiary() } else { mac::text() })
                    .when(!disabled, |el: Stateful<gpui::Div>| {
                        el.hover(|h| h.bg(mac::chrome())).on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_column(key, cx);
                        }))
                    })
                    .child(
                        div()
                            .w(px(14.0))
                            .text_color(gpui::rgb(0x007aff))
                            .child(if on { "✓" } else { "" }),
                    )
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
                    .text_size(px(11.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text_tertiary())
                    .child(label.to_uppercase()),
            )
            .child(
                div()
                    .text_size(px(24.0))
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
            .child(div().flex().items_end().gap(px(1.0)).size_full().children(bars))
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
                    .text_size(px(11.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text_tertiary())
                    .child("CPU CORES"),
            )
            .child(
                div().flex().flex_wrap().gap_x_4().gap_y_2().children(
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
                                    .text_size(px(11.0))
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
                                    .text_size(px(11.0))
                                    .text_color(mac::text())
                                    .child(format!("{:.0}%", usage)),
                            )
                    }),
                ),
            )
    }

    /// The Network tab body: a real per-interface table (sysinfo `Networks`)
    /// since there is no reliable per-process network data on this platform.
    fn render_network_pane(&self) -> impl IntoElement {
        let teal = gpui::rgb(0x32ade6);
        // Column widths (interface label flexes, figures are fixed/right-aligned).
        let figure = |s: String, color: gpui::Hsla| {
            div()
                .w(px(110.0))
                .text_size(px(12.0))
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
                    .text_size(px(11.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text_tertiary())
                    .child("INTERFACE"),
            )
            .children(["RCVD", "SENT", "↓ RATE", "↑ RATE"].into_iter().map(|h| {
                div()
                    .w(px(110.0))
                    .text_size(px(11.0))
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
                            .child(
                                div()
                                    .size(px(7.0))
                                    .rounded_full()
                                    .bg(if active { teal.into() } else { mac::text_tertiary() }),
                            )
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .text_color(mac::text())
                                    .child(iface.name.clone()),
                            ),
                    )
                    .child(figure(format_bytes(iface.total_recv), mac::text()))
                    .child(figure(format_bytes(iface.total_sent), mac::text()))
                    .child(figure(
                        format_rate(iface.recv_rate),
                        if active { teal.into() } else { mac::text_secondary() },
                    ))
                    .child(figure(
                        format_rate(iface.sent_rate),
                        if active { teal.into() } else { mac::text_secondary() },
                    ))
                    .into_any_element()
            })
            .collect();

        div()
            .flex_1()
            .min_h(px(0.0))
            .px_4()
            .pb_4()
            .child(
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

        let (cards, samples, accent): (Vec<gpui::AnyElement>, &[f32], gpui::Hsla) = match self.tab
        {
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
                    self.stat_card("CPU Load", format!("{:.1}%", self.agg.cpu_total), mac::text())
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
            .when(matches!(self.tab, Tab::Cpu) && !self.agg.per_core.is_empty(), |el| {
                el.child(self.render_core_bars())
            })
            .when(matches!(self.tab, Tab::Memory) && self.agg.mem_total > 0, |el| {
                el.child(self.render_mem_pressure())
            })
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
                            .text_size(px(11.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text_tertiary())
                            .child("MEMORY PRESSURE"),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
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
        let group = ButtonGroup::new("tabs")
            .outline()
            .children(Tab::ALL.iter().map(|t| {
                Button::new(SharedString::from(t.label()))
                    .label(t.label())
                    .selected(*t == active)
            }))
            .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                if let Some(&ix) = clicks.first() {
                    this.select_tab(Tab::ALL[ix], cx);
                }
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
                        Button::new("quit")
                            .label("Quit")
                            .disabled(!has_sel)
                            .on_click(cx.listener(|this, _, _, cx| this.request_kill(false, cx))),
                    )
                    .child(
                        Button::new("force-quit")
                            .label("Force Quit")
                            .danger()
                            .disabled(!has_sel)
                            .on_click(cx.listener(|this, _, _, cx| this.request_kill(true, cx))),
                    )
                    .when(self.tab.has_process_table(), |el| {
                        el.child(
                            Button::new("columns")
                                .label("Columns")
                                .selected(self.cols_menu_open)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cols_menu_open = !this.cols_menu_open;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(div().w(px(220.0)).child(Input::new(&self.search).small())),
            )
    }

    fn render_confirm(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let p = self.pending_kill.clone()?;
        let verb = if p.force { "Force Quit" } else { "Quit" };
        let body = format!(
            "Do you want to {} the process \u{201c}{}\u{201d} (PID {})?",
            verb.to_lowercase(),
            p.name,
            p.pid
        );
        let dialog = div()
            .v_flex()
            .gap_4()
            .w(px(380.0))
            .p_5()
            .rounded(px(12.0))
            .bg(mac::window())
            .border_1()
            .border_color(mac::separator())
            .shadow_lg()
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text())
                    .child(format!("{verb} Process")),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(mac::text_secondary())
                    .child(body),
            )
            .child(
                div()
                    .h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel")
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_kill(cx))),
                    )
                    .child(
                        Button::new("confirm")
                            .label(verb)
                            .when(p.force, |b| b.danger())
                            .when(!p.force, |b| b.primary())
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_kill(cx))),
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
                .bg(gpui::rgba(0x00000040))
                .child(dialog),
        )
    }

    /// The double-click process inspector — a detail panel of real `sysinfo` data.
    fn render_inspector(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let pid = self.inspect_pid?;
        let state = self.table.read(cx);
        let d = state.delegate();
        let row = d.rows.iter().chain(d.all_rows.iter()).find(|r| r.pid == pid)?;
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
                .child(div().text_size(px(12.0)).text_color(mac::text_secondary()).child(label.to_string()))
                .child(
                    div()
                        .text_size(px(12.0))
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
                            .text_size(px(16.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text())
                            .child(row.name.clone()),
                    )
                    .child(
                        Button::new("inspect-close")
                            .label("Done")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.inspect_pid = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(info_row("Process ID (PID)", row.pid.to_string()))
            .child(info_row("Parent PID", row.ppid.map(|p| p.to_string()).unwrap_or_else(|| "—".into())))
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
                    .child(div().text_size(px(12.0)).text_color(mac::text_secondary()).child("Path"))
                    .child(
                        div()
                            .text_size(px(11.0))
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
                .bg(gpui::rgba(0x00000040))
                .child(dialog),
        )
    }
}

impl Render for MonitorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .key_context("ActivityMonitor")
            .on_action(cx.listener(|this, _: &QuitProcess, _, cx| this.request_kill(false, cx)))
            .on_action(cx.listener(|this, _: &ForceQuitProcess, _, cx| this.request_kill(true, cx)))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)))
            .on_action(cx.listener(|this, _: &ConfirmKill, _, cx| this.confirm_kill(cx)))
            .on_action(cx.listener(|this, _: &CancelKill, _, cx| this.cancel_kill(cx)))
            .on_action(cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()))
            .size_full()
            .v_flex()
            .bg(mac::window())
            .text_color(mac::text())
            .child(rmac_ui::title_bar("Activity Monitor"))
            .child(self.render_toolbar(cx))
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
            .when(self.cols_menu_open && self.tab.has_process_table(), |this| {
                this.child(self.render_columns_menu(cx))
            })
            .children(self.render_confirm(cx))
            .children(self.render_inspector(cx))
    }
}

fn main() {
    rmac_ui::boot("Activity Monitor", 1040.0, 680.0, |window, cx| {
        let view = MonitorView::new(window, cx);
        cx.bind_keys([
            gpui::KeyBinding::new("cmd-f", FocusSearch, Some("ActivityMonitor")),
            gpui::KeyBinding::new("cmd-backspace", QuitProcess, Some("ActivityMonitor")),
            gpui::KeyBinding::new("shift-cmd-backspace", ForceQuitProcess, Some("ActivityMonitor")),
            gpui::KeyBinding::new("enter", ConfirmKill, Some("ActivityMonitor")),
            gpui::KeyBinding::new("escape", CancelKill, Some("ActivityMonitor")),
        ]);
        window.focus(&view.focus);
        view
    });
}
