//! rmac Activity Monitor — a fast, native, GPU-rendered process monitor.
//!
//! Phase 1 of the rmac desktop suite. Built on GPUI + gpui-component, with
//! `sysinfo` as the data layer. Runs on macOS today (Metal) and targets
//! Ubuntu/Wayland (Vulkan) later — the same binary, no webview, instant launch.

use std::cmp::Ordering;
use std::time::Duration;

use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, Context, Entity,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement, Render, SharedString,
    Stateful, Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonGroup, ButtonVariants as _},
    input::{Input, InputState},
    menu::PopupMenu,
    table::{Column, ColumnSort, Table, TableDelegate, TableState},
    Disableable as _, Selectable as _, Sizable as _, StyledExt as _,
};
use rmac_ui::mac;
use sysinfo::{Networks, Pid, ProcessesToUpdate, Signal, System};

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
    fn default_sort_col(self) -> usize {
        match self {
            Tab::Cpu => 2,
            Tab::Memory => 3,
            Tab::Energy => 4,
            Tab::Disk => 5,
            Tab::Network => 2,
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
    columns: Vec<Column>,
    cpu_count: usize,
    filter: String,
    sort_col: usize,
    sort_asc: bool,
}

impl ProcessTableDelegate {
    fn new() -> Self {
        let mut delegate = Self {
            system: System::new_all(),
            all_rows: Vec::new(),
            rows: Vec::new(),
            columns: vec![
                Column::new("pid", "PID").width(px(72.0)).text_right().sortable(),
                Column::new("name", "Process").width(px(280.0)).sortable(),
                Column::new("cpu", "% CPU").width(px(96.0)).text_right().sortable(),
                Column::new("mem", "Memory").width(px(110.0)).text_right().sortable(),
                Column::new("energy", "Energy").width(px(96.0)).text_right().sortable(),
                Column::new("disk", "Disk I/O").width(px(120.0)).text_right().sortable(),
            ],
            cpu_count: 1,
            filter: String::new(),
            // Default: busiest CPU first.
            sort_col: 2,
            sort_asc: false,
        };
        delegate.refresh();
        delegate
    }

    /// Pull a fresh snapshot from `sysinfo`, then re-apply the active filter/sort.
    fn refresh(&mut self) {
        self.system.refresh_cpu_all();
        self.system.refresh_memory();
        self.system
            .refresh_processes(ProcessesToUpdate::All, true);
        self.cpu_count = self.system.cpus().len().max(1);

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
                ProcRow {
                    pid: p.pid().as_u32(),
                    name: p.name().to_string_lossy().into_owned().into(),
                    cmd_search,
                    cpu: p.cpu_usage(),
                    mem: p.memory(),
                    disk: du.read_bytes + du.written_bytes,
                    energy: p.cpu_usage(),
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
        Self::sort_rows(&mut rows, self.sort_col, self.sort_asc);
        rows.truncate(300);
        self.rows = rows;
    }

    fn sort_rows(rows: &mut [ProcRow], col: usize, asc: bool) {
        rows.sort_by(|a, b| {
            let o = match col {
                0 => a.pid.cmp(&b.pid),
                1 => a.name.cmp(&b.name),
                2 => a.cpu.partial_cmp(&b.cpu).unwrap_or(Ordering::Equal),
                3 => a.mem.cmp(&b.mem),
                4 => a.energy.partial_cmp(&b.energy).unwrap_or(Ordering::Equal),
                5 => a.disk.cmp(&b.disk),
                _ => Ordering::Equal,
            };
            if asc {
                o
            } else {
                o.reverse()
            }
        });
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
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) {
        self.sort_col = col_ix;
        self.sort_asc = matches!(sort, ColumnSort::Ascending);
        self.apply_view();
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
                MouseButton::Right,
                cx.listener(move |state, _, _, cx| {
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
        let text: SharedString = match col_ix {
            0 => row.pid.to_string().into(),
            1 => row.name.clone(),
            2 => format!("{:.1}", row.cpu).into(),
            3 => format_mem(row.mem).into(),
            4 => format!("{:.1}", row.energy).into(),
            5 => format_mem(row.disk).into(),
            _ => "".into(),
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

        let mut view = Self {
            table,
            search,
            focus: cx.focus_handle(),
            networks: Networks::new_with_refreshed_list(),
            tab: Tab::Cpu,
            agg: Aggregates::default(),
            history: History::default(),
            pending_kill: None,
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
            state.refresh(cx);
        });
        cx.notify();
    }

    /// Refresh the table snapshot and recompute the summary aggregates.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.networks.refresh(true);
        let (net_recv, net_sent) = self
            .networks
            .list()
            .values()
            .fold((0u64, 0u64), |(r, t), d| {
                (r + d.received(), t + d.transmitted())
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
        self.table.update(cx, |state, cx| {
            let d = state.delegate_mut();
            d.sort_col = tab.default_sort_col();
            d.sort_asc = false;
            d.apply_view();
            state.refresh(cx);
        });
        cx.notify();
    }

    fn selected_proc(&self, cx: &Context<Self>) -> Option<(u32, SharedString)> {
        let state = self.table.read(cx);
        let ix = state.selected_row()?;
        let row = state.delegate().rows.get(ix)?;
        Some((row.pid, row.name.clone()))
    }

    fn request_kill(&mut self, force: bool, cx: &mut Context<Self>) {
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
        if self.pending_kill.take().is_some() {
            cx.notify();
        }
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.focus(window, cx));
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

    fn render_summary(&self, cx: &Context<Self>) -> impl IntoElement {
        let blue = gpui::rgb(0x007aff).into();
        let green = gpui::rgb(0x28b463).into();
        let orange = gpui::rgb(0xff9500).into();
        let purple = gpui::rgb(0xaf52de).into();
        let teal = gpui::rgb(0x32ade6).into();

        let (cards, samples, accent): (Vec<gpui::AnyElement>, &[f32], gpui::Hsla) = match self.tab
        {
            Tab::Cpu => {
                let cards = vec![
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
            .size_full()
            .v_flex()
            .bg(mac::window())
            .text_color(mac::text())
            .child(rmac_ui::title_bar("Activity Monitor"))
            .child(self.render_toolbar(cx))
            .child(self.render_summary(cx))
            .child(
                // The live table fills the rest
                div()
                    .flex_1()
                    .px_4()
                    .pb_4()
                    .child(Table::new(&self.table).stripe(true).bordered(true)),
            )
            .children(self.render_confirm(cx))
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
