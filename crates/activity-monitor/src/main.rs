//! rmac Activity Monitor — a fast, native, GPU-rendered process monitor.
//!
//! Phase 1 of the rmac desktop suite. Built on GPUI + gpui-component, with
//! `sysinfo` as the data layer. Runs on macOS today (Metal) and targets
//! Ubuntu/Wayland (Vulkan) later — the same binary, no webview, instant launch.

use std::cmp::Ordering;
use std::time::Duration;

use gpui::{
    div, point, px, size, App, Application, Bounds, Context, Entity, IntoElement, ParentElement,
    Render, SharedString, Styled, Window, WindowBounds, WindowOptions,
};
use gpui_component::{
    table::{Column, ColumnSort, Table, TableDelegate, TableState},
    ActiveTheme as _, Root, StyledExt as _,
};
use sysinfo::{ProcessesToUpdate, System};

/// One row in the process table — a flat snapshot, cheap to clone/diff.
#[derive(Clone)]
struct ProcRow {
    pid: u32,
    name: SharedString,
    cpu: f32,
    mem: u64,
}

/// Table delegate: owns the live `System` handle plus the current snapshot.
///
/// We keep a single `System` instance and refresh it in place so `sysinfo`
/// can compute per-process CPU deltas between ticks (it works on diffs).
struct ProcessTableDelegate {
    system: System,
    rows: Vec<ProcRow>,
    columns: Vec<Column>,
    cpu_count: usize,
}

impl ProcessTableDelegate {
    fn new() -> Self {
        let mut delegate = Self {
            system: System::new_all(),
            rows: Vec::new(),
            columns: vec![
                Column::new("pid", "PID").width(px(80.0)).text_right(),
                Column::new("name", "Process").width(px(360.0)).sortable(),
                Column::new("cpu", "% CPU").width(px(120.0)).text_right().sortable(),
                Column::new("mem", "Memory").width(px(140.0)).text_right().sortable(),
            ],
            cpu_count: 1,
        };
        delegate.refresh();
        delegate
    }

    /// Pull a fresh snapshot, sort by CPU descending, keep the busiest 200.
    fn refresh(&mut self) {
        self.system.refresh_cpu_all();
        self.system.refresh_processes(ProcessesToUpdate::All, true);
        self.cpu_count = self.system.cpus().len().max(1);

        let mut rows: Vec<ProcRow> = self
            .system
            .processes()
            .values()
            .map(|p| ProcRow {
                pid: p.pid().as_u32(),
                name: p.name().to_string_lossy().into_owned().into(),
                cpu: p.cpu_usage(),
                mem: p.memory(),
            })
            .collect();

        rows.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(Ordering::Equal));
        rows.truncate(200);
        self.rows = rows;
    }
}

fn format_mem(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else {
        format!("{:.1} MB", b / MB)
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
        let asc = matches!(sort, ColumnSort::Ascending);
        match col_ix {
            1 => self.rows.sort_by(|a, b| {
                let o = a.name.cmp(&b.name);
                if asc { o } else { o.reverse() }
            }),
            2 => self.rows.sort_by(|a, b| {
                let o = a.cpu.partial_cmp(&b.cpu).unwrap_or(Ordering::Equal);
                if asc { o } else { o.reverse() }
            }),
            3 => self.rows.sort_by(|a, b| {
                let o = a.mem.cmp(&b.mem);
                if asc { o } else { o.reverse() }
            }),
            _ => {}
        }
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
            _ => "".into(),
        };
        div().child(text)
    }
}

/// Root view: a summary strip + the live process table.
struct MonitorView {
    table: Entity<TableState<ProcessTableDelegate>>,
    cpu_total: f32,
    mem_used: u64,
    mem_total: u64,
}

impl MonitorView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| TableState::new(ProcessTableDelegate::new(), window, cx));

        let mut view = Self {
            table,
            cpu_total: 0.0,
            mem_used: 0,
            mem_total: 0,
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

    /// Refresh the table snapshot and recompute the summary aggregates.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |state, cx| {
            let delegate = state.delegate_mut();
            delegate.refresh();
            let count = delegate.cpu_count as f32;
            // Sum of per-process CPU is ~total busy across all cores; normalise to 0-100%.
            self.cpu_total =
                (delegate.rows.iter().map(|r| r.cpu).sum::<f32>() / count).min(100.0);
            self.mem_used = delegate.system.used_memory();
            self.mem_total = delegate.system.total_memory();
            state.refresh(cx);
        });
    }

    fn stat_card(&self, label: &str, value: String, cx: &Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .gap_1()
            .px_4()
            .py_3()
            .rounded(px(10.0))
            .bg(cx.theme().secondary)
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(label.to_string()),
            )
            .child(div().text_xl().child(value))
    }
}

impl Render for MonitorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mem_label = format!(
            "{} / {}",
            format_mem(self.mem_used),
            format_mem(self.mem_total)
        );
        let proc_count = self.table.read(cx).delegate().rows.len();

        div()
            .size_full()
            .v_flex()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                // Summary strip
                div()
                    .h_flex()
                    .gap_3()
                    .p_4()
                    .child(self.stat_card("CPU Load", format!("{:.1}%", self.cpu_total), cx))
                    .child(self.stat_card("Memory", mem_label, cx))
                    .child(self.stat_card("Processes", proc_count.to_string(), cx)),
            )
            .child(
                // The live table fills the rest
                div()
                    .flex_1()
                    .px_4()
                    .pb_4()
                    .child(Table::new(&self.table).stripe(true).bordered(true)),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        gpui_component::init(cx);

        let bounds = Bounds::new(point(px(200.0), px(120.0)), size(px(960.0), px(640.0)));

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|cx| MonitorView::new(window, cx));
                cx.new(|cx| Root::new(view.into(), window, cx))
            },
        )
        .unwrap();

        cx.activate(true);
    });
}
