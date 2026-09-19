use std::cmp::Ordering;

use gpui::{
    div, App, Context, InteractiveElement as _, IntoElement, MouseButton, ParentElement,
    SharedString, Stateful, Window,
};
use gpui_component::menu::PopupMenu;
use rmac_activity_monitor::accessibility::{ProcessColumn, ProcessRowSemantics};
use rmac_ui::{Column, ColumnSort, TableDelegate, TableState};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind, Users};

use crate::columns::ColKey;
use crate::metrics::{format_duration, format_mem};
use crate::{ForceQuitProcess, QuitProcess};

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
}

impl ProcRow {
    fn cell_text(&self, key: ColKey) -> String {
        match key {
            ColKey::Pid => self.pid.to_string(),
            ColKey::Name => self.name.to_string(),
            ColKey::Cpu => format!("{:.1}", self.cpu),
            ColKey::Mem => format_mem(self.mem),
            ColKey::Energy => format!("{:.1}", self.energy),
            ColKey::Disk => format_mem(self.disk),
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
    pub(crate) cpu_count: usize,
    filter: String,
    /// The column the rows are sorted by — identity-based so it survives the
    /// visible set changing under it.
    pub(crate) sort_key: ColKey,
    pub(crate) sort_asc: bool,
    /// PID is the selection source of truth because row indexes drift whenever
    /// the table is refreshed or sorted.
    pub(crate) selected_pid: Option<u32>,
}

impl ProcessTableDelegate {
    pub(crate) fn new(visible: Vec<ColKey>) -> Self {
        let mut delegate = Self {
            system: System::new(),
            all_rows: Vec::new(),
            rows: Vec::new(),
            columns: visible.iter().map(|key| key.to_column()).collect(),
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
        true
    }

    pub(crate) fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.apply_view();
    }

    /// Pull a fresh snapshot from `sysinfo`, then reapply the active view.
    pub(crate) fn refresh(&mut self) {
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
        self.cpu_count = self.system.cpus().len().max(1);

        let users = &self.users;
        self.all_rows = self
            .system
            .processes()
            .values()
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
                let user = process
                    .user_id()
                    .and_then(|uid| users.get_user_by_id(uid))
                    .map(|user| SharedString::from(user.name().to_string()))
                    .or_else(|| {
                        process
                            .user_id()
                            .map(|uid| SharedString::from(format!("uid {}", **uid)))
                    })
                    .unwrap_or_else(|| SharedString::from("—"));
                ProcRow {
                    pid: process.pid().as_u32(),
                    name: process.name().to_string_lossy().into_owned().into(),
                    cmd_search: cmd_search.into(),
                    cpu,
                    mem: process.memory(),
                    disk,
                    energy: cpu + (disk as f32 / 1_048_576.0) * 0.5,
                    ppid: process.parent().map(|parent| parent.as_u32()),
                    user,
                    vmem: process.virtual_memory(),
                    run_time: process.run_time(),
                    start_time: process.start_time(),
                    status: SharedString::from(process.status().to_string()),
                }
            })
            .collect();

        self.apply_view();
    }

    /// Rebuild the bounded visible rows from the complete snapshot.
    pub(crate) fn apply_view(&mut self) {
        let needle = self.filter.to_lowercase();
        Self::sort_rows(&mut self.all_rows, self.sort_key, self.sort_asc);
        self.rows = self
            .all_rows
            .iter()
            .filter(|row| {
                needle.is_empty()
                    || row.name.to_lowercase().contains(&needle)
                    || row.pid.to_string().contains(&needle)
                    || row.cmd_search.contains(&needle)
            })
            .take(300)
            .cloned()
            .collect();
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
            };
            if ascending {
                ordering
            } else {
                ordering.reverse()
            }
        });
    }
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
    state.delegate_mut().selected_pid = retained_pid;
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

    fn render_tr(
        &mut self,
        row_index: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<gpui::Div> {
        div()
            .id(("row", row_index))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |state, _, _, cx| {
                    let pid = state.delegate().rows.get(row_index).map(|row| row.pid);
                    state.delegate_mut().selected_pid = pid;
                    state.set_selected_row(row_index, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |state, _, _, cx| {
                    let pid = state.delegate().rows.get(row_index).map(|row| row.pid);
                    state.delegate_mut().selected_pid = pid;
                    state.set_selected_row(row_index, cx);
                }),
            )
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
        if let Some(row) = self.rows.get(row_index) {
            self.selected_pid = Some(row.pid);
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

#[cfg(test)]
mod tests {
    use super::selection_projection;

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
}
