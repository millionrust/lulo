//! Window ▸ Show Progress Window: one app-wide window listing every file
//! operation running in any Files window, each with its live progress and a
//! Stop control.
//!
//! It has no tracking of its own. Each row is read from the same
//! `FinderView` state the in-window progress bars draw (`transfer`,
//! `undo_operation`, `trash_operation`, `archive_job`), the window observes
//! every Files window so it redraws when they do, and Stop calls the same
//! cancel method as each bar's own Cancel button.

use super::*;

const WIDTH: f32 = 440.0;
const HEIGHT: f32 = 260.0;
const EMPTY_STATE: &str = "No operations in progress";

/// The kinds of long-running work a Files window reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OperationKind {
    /// Copy, move or duplicate (`FinderView::transfer`).
    Transfer,
    /// Edit ▸ Undo of a file operation.
    Undo,
    /// Move to Trash, Put Back or Delete Immediately.
    Trash,
    /// Compress or expand.
    Archive,
}

#[derive(Clone, Debug, PartialEq)]
struct ProgressRow {
    kind: OperationKind,
    title: String,
    status: String,
    /// `None` while the total is not yet known.
    fraction: Option<f32>,
    stopping: bool,
}

fn fraction(done: u64, total: u64) -> Option<f32> {
    (total > 0).then(|| (done as f32 / total as f32).clamp(0.0, 1.0))
}

/// The progress helpers' text without the operation title in front of it,
/// since the window prints the title on its own line.
fn status_without_title(text: String, title: &str) -> String {
    text.strip_prefix(title)
        .and_then(|rest| rest.strip_prefix(" — "))
        .map(str::to_owned)
        .unwrap_or(text)
}

fn transfer_fraction(transfer: &ActiveTransfer) -> Option<f32> {
    match transfer.phase {
        file_ops::TransferPhase::Scanning => None,
        file_ops::TransferPhase::Copying if transfer.bytes_total > 0 => fraction(
            transfer.bytes_processed,
            transfer.bytes_total.max(transfer.bytes_processed),
        ),
        file_ops::TransferPhase::Copying | file_ops::TransferPhase::Finishing => {
            fraction(transfer.processed as u64, transfer.total as u64)
        }
    }
}

/// One Files window's running operations, in the order its in-window bars
/// stack. `trash` is `(label, processed, total, cancelling)`, as the window
/// already derives it for its own bar.
fn operation_rows(
    transfer: Option<&ActiveTransfer>,
    undo: Option<&ActiveUndo>,
    trash: Option<(SharedString, usize, usize, bool)>,
    archive: Option<&archive_controller::ArchiveJob>,
) -> Vec<ProgressRow> {
    let mut rows = Vec::new();
    if let Some((label, processed, total, cancelling)) = trash {
        rows.push(ProgressRow {
            kind: OperationKind::Trash,
            title: label.to_string(),
            status: status_without_title(
                accessibility::trash_progress_text(label.as_ref(), processed, total),
                label.as_ref(),
            ),
            fraction: fraction(processed as u64, total as u64),
            stopping: cancelling,
        });
    }
    if let Some(undo) = undo {
        rows.push(ProgressRow {
            kind: OperationKind::Undo,
            title: undo.label.to_string(),
            status: status_without_title(
                accessibility::undo_progress_text(undo),
                undo.label.as_ref(),
            ),
            fraction: None,
            stopping: undo.cancelling,
        });
    }
    if let Some(transfer) = transfer {
        rows.push(ProgressRow {
            kind: OperationKind::Transfer,
            title: transfer.label.to_string(),
            status: status_without_title(
                accessibility::transfer_progress_text(transfer),
                transfer.label.as_ref(),
            ),
            fraction: transfer_fraction(transfer),
            stopping: transfer.cancelling,
        });
    }
    if let Some(job) = archive {
        rows.push(ProgressRow {
            kind: OperationKind::Archive,
            title: job.label.to_string(),
            status: rmac_archive::progress_line(job.progress, job.started.elapsed()),
            fraction: fraction(job.progress.done, job.progress.total),
            stopping: job.cancel.load(Ordering::Acquire),
        });
    }
    rows
}

impl FinderView {
    fn progress_rows(&self) -> Vec<ProgressRow> {
        #[cfg(any(target_os = "linux", test))]
        let trash = self.trash_operation.as_ref().map(|operation| {
            (
                SharedString::from(bin_copy(self.file_words, operation.label.as_ref())),
                operation.processed,
                operation.total,
                operation.cancelling,
            )
        });
        #[cfg(not(any(target_os = "linux", test)))]
        let trash: Option<(SharedString, usize, usize, bool)> = None;
        operation_rows(
            self.transfer.as_ref(),
            self.undo_operation.as_ref(),
            trash,
            self.archive_job.as_ref(),
        )
    }

    /// Stop from the Progress window: the same cancel as the operation's own
    /// in-window Cancel button.
    fn stop_operation(&mut self, kind: OperationKind, cx: &mut Context<Self>) {
        match kind {
            OperationKind::Transfer => self.cancel_transfer(cx),
            OperationKind::Undo => self.cancel_undo(cx),
            OperationKind::Trash => self.cancel_trash(cx),
            OperationKind::Archive => self.cancel_archive_job(cx),
        }
    }

    /// Window ▸ Show Progress Window. Deferred so the window's first frame,
    /// which reads every Files window, never runs inside this one's update.
    pub(super) fn show_progress_window(&mut self, cx: &mut Context<Self>) {
        cx.defer(show_progress_window);
    }
}

/// The open Progress window, shared by every Files window.
#[derive(Default)]
struct OpenProgressWindow(Option<gpui::WindowHandle<Root>>);

impl gpui::Global for OpenProgressWindow {}

fn show_progress_window(cx: &mut gpui::App) {
    let open = cx
        .try_global::<OpenProgressWindow>()
        .and_then(|open| open.0);
    if let Some(handle) = open {
        if cx
            .update_window(*handle, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    let (width, height) = rmac_ui::outer_window_size(WIDTH, HEIGHT);
    // Not `window_options_for_app_with_title`: Progress would then inherit
    // whatever size the main Files window last saved under the same
    // app_id (the UIA-06/UIA-09 window-geometry-key bug).
    let mut options = rmac_ui::window_options_for_panel_with_title(
        rmac_ui::app_id::FILES,
        "Progress",
        width,
        height,
        cx,
    );
    options.window_min_size = Some(gpui::size(px(width), px(height)));
    let opened = cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| ProgressWindow::new(window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    });
    match opened {
        Ok(handle) => cx.set_global(OpenProgressWindow(Some(handle))),
        Err(error) => eprintln!("rmac-files: could not open the Progress window: {error}"),
    }
}

struct ProgressWindow {
    focus: FocusHandle,
    /// Every Files window, observed so this window redraws with their bars.
    observed: Vec<(gpui::WeakEntity<FinderView>, [gpui::Subscription; 2])>,
    _new_windows: gpui::Subscription,
}

impl ProgressWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        window.set_window_title("Progress");
        let progress = cx.weak_entity();
        let new_windows = cx.observe_new::<FinderView>(move |_, _, cx| {
            let finder = cx.entity();
            let _ = progress.update(cx, |this, cx| this.observe(finder, cx));
        });
        let mut this = Self {
            focus: cx.focus_handle(),
            observed: Vec::new(),
            _new_windows: new_windows,
        };
        for weak in super::settings::finder_windows(cx) {
            if let Some(finder) = weak.upgrade() {
                this.observe(finder, cx);
            }
        }
        this
    }

    fn observe(&mut self, finder: Entity<FinderView>, cx: &mut Context<Self>) {
        self.observed.retain(|(weak, _)| weak.upgrade().is_some());
        if self.observed.iter().any(|(weak, _)| *weak == finder) {
            return;
        }
        let subscriptions = [
            cx.observe(&finder, |_, _, cx| cx.notify()),
            cx.observe_release(&finder, |_, _, cx| cx.notify()),
        ];
        self.observed.push((finder.downgrade(), subscriptions));
        cx.notify();
    }
}

impl Render for ProgressWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app: &gpui::App = cx;
        let rows: Vec<(gpui::WeakEntity<FinderView>, ProgressRow)> = self
            .observed
            .iter()
            .filter_map(|(weak, _)| weak.upgrade().map(|finder| (weak.clone(), finder)))
            .flat_map(|(weak, finder)| {
                finder
                    .read(app)
                    .progress_rows()
                    .into_iter()
                    .map(move |row| (weak.clone(), row))
            })
            .collect();
        let list = if rows.is_empty() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(rmac_ui::mac::text_secondary())
                .child(EMPTY_STATE)
                .into_any_element()
        } else {
            div()
                .id("progress-operations")
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .children(rows.into_iter().enumerate().map(|(index, (owner, row))| {
                    let kind = row.kind;
                    let progress = match row.fraction {
                        Some(fraction) => rmac_ui::Progress::new(fraction),
                        None => rmac_ui::Progress::indeterminate(),
                    };
                    let status = if row.stopping {
                        "Stopping…".to_owned()
                    } else {
                        row.status
                    };
                    div()
                        .id(SharedString::from(format!("progress-row-{index}")))
                        .role(Role::Group)
                        .aria_label(row.title.clone())
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_4()
                        .py_3()
                        .border_b_1()
                        .border_color(rmac_ui::mac::separator())
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(rmac_ui::text_px(13.0))
                                        .text_color(rmac_ui::mac::text())
                                        .child(row.title),
                                )
                                .child(progress)
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(rmac_ui::text_px(11.0))
                                        .text_color(rmac_ui::mac::text_secondary())
                                        .child(status),
                                ),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("progress-stop-{index}")))
                                .role(Role::Button)
                                .aria_label("Stop")
                                .flex_none()
                                .size(px(16.0))
                                .cursor_pointer()
                                .when(row.stopping, |stop| stop.opacity(0.4))
                                .child(icon(
                                    "icons/quick-look/close.svg",
                                    16.0,
                                    rmac_ui::mac::text_secondary(),
                                ))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    let _ = owner
                                        .update(cx, |finder, cx| finder.stop_operation(kind, cx));
                                })),
                        )
                }))
                .into_any_element()
        };
        div()
            .track_focus(&self.focus)
            .size_full()
            .v_flex()
            .bg(rmac_ui::mac::window())
            .child(rmac_ui::title_bar_content(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.0))
                    .font_weight(rmac_ui::mac::BOLD)
                    .text_color(rmac_ui::mac::text_secondary())
                    .child("Progress"),
            ))
            .child(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transfer(phase: file_ops::TransferPhase) -> ActiveTransfer {
        ActiveTransfer {
            label: "Copying 3 items to Documents".into(),
            phase,
            processed: 1,
            total: 4,
            bytes_processed: 256,
            bytes_total: 1024,
            cancel: Arc::new(AtomicBool::new(false)),
            cancelling: false,
            keep_unfinished_in_clipboard: false,
            retained_clipboard: Vec::new(),
            play_drop_sound: false,
        }
    }

    #[test]
    fn no_running_operation_lists_no_rows() {
        assert!(operation_rows(None, None, None, None).is_empty());
    }

    #[test]
    fn transfer_progress_follows_bytes_then_items() {
        let scanning = transfer(file_ops::TransferPhase::Scanning);
        assert_eq!(transfer_fraction(&scanning), None);

        let copying = transfer(file_ops::TransferPhase::Copying);
        assert_eq!(transfer_fraction(&copying), Some(0.25));

        let mut without_bytes = transfer(file_ops::TransferPhase::Copying);
        without_bytes.bytes_total = 0;
        without_bytes.processed = 2;
        assert_eq!(transfer_fraction(&without_bytes), Some(0.5));

        let mut finishing = transfer(file_ops::TransferPhase::Finishing);
        finishing.processed = 4;
        assert_eq!(transfer_fraction(&finishing), Some(1.0));
    }

    #[test]
    fn rows_carry_each_operations_title_status_and_stop_state() {
        let mut copying = transfer(file_ops::TransferPhase::Copying);
        copying.cancelling = true;
        let rows = operation_rows(
            Some(&copying),
            None,
            Some(("Moving to Trash".into(), 2, 8, false)),
            None,
        );

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, OperationKind::Trash);
        assert_eq!(rows[0].title, "Moving to Trash");
        assert_eq!(rows[0].status, "2 of 8 items");
        assert_eq!(rows[0].fraction, Some(0.25));
        assert!(!rows[0].stopping);

        assert_eq!(rows[1].kind, OperationKind::Transfer);
        assert_eq!(rows[1].title, "Copying 3 items to Documents");
        assert_eq!(rows[1].status, "1 of 4 · 256 bytes of 1 KB");
        assert!(rows[1].stopping);
    }

    #[test]
    fn status_keeps_text_that_does_not_start_with_the_title() {
        assert_eq!(
            status_without_title("Copying — 1 of 2".to_owned(), "Copying"),
            "1 of 2"
        );
        assert_eq!(
            status_without_title("Something else".to_owned(), "Copying"),
            "Something else"
        );
    }

    #[test]
    fn fraction_is_unknown_without_a_total_and_clamped_with_one() {
        assert_eq!(fraction(5, 0), None);
        assert_eq!(fraction(5, 10), Some(0.5));
        assert_eq!(fraction(20, 10), Some(1.0));
    }
}
