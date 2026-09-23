//! Archive Utility inside Files: double-click expands archives next to
//! themselves, File ▸ Compress writes "name.zip" / "Archive.zip", both with
//! the Mac's names, a progress sheet for slow jobs and the Mac's alert text
//! (crates/rmac-archive; design-lab/archive.html).

use std::time::Instant;

use super::*;

/// The Mac shows nothing for quick jobs; the sheet appears only once a job
/// has run this long (S: measured as "no window" for a fast 1.2 GB stored
/// zip and a window for a 20 s compress).
const SHEET_DELAY: Duration = Duration::from_secs(1);

pub(super) struct ArchiveJob {
    pub(super) generation: u64,
    pub(super) label: SharedString,
    pub(super) progress: rmac_archive::Progress,
    pub(super) started: Instant,
    pub(super) visible: bool,
    pub(super) cancel: Arc<AtomicBool>,
}

enum ArchiveWork {
    Expand(Vec<PathBuf>),
    Compress(Vec<PathBuf>),
}

impl ArchiveWork {
    /// Run to completion; the alert text on failure (`None` when it worked
    /// or was cancelled).
    fn run(
        self,
        cancel: &AtomicBool,
        report: &mut dyn FnMut(rmac_archive::Progress),
    ) -> Option<String> {
        match self {
            Self::Expand(archives) => {
                for archive in archives {
                    if let Err(error) = rmac_archive::expand(&archive, cancel, report) {
                        return rmac_archive::expand_error_message(&archive, &error);
                    }
                }
                None
            }
            Self::Compress(items) => rmac_archive::compress(&items, cancel, report)
                .err()
                .and_then(|error| rmac_archive::compress_error_message(&items, &error)),
        }
    }
}

fn quoted_name(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    format!("“{}”", sanitize_dialog_name(&name))
}

/// "Compress “notes.txt”" / "Compress 2 Items", as Finder's menus say it.
pub(super) fn compress_menu_label(paths: &[PathBuf]) -> Option<String> {
    match paths {
        [] => None,
        [one] => Some(format!("Compress {}", quoted_name(one))),
        _ => Some(format!("Compress {} Items", paths.len())),
    }
}

impl FinderView {
    pub(super) fn compress_menu_label(&self) -> Option<String> {
        compress_menu_label(&self.selected_paths())
    }

    /// Double-click (or Uncompress in Quick Look) on archives.
    pub(super) fn expand_archives(&mut self, archives: Vec<PathBuf>, cx: &mut Context<Self>) {
        let label = match archives.as_slice() {
            [] => return,
            [one] => format!("Expanding {}", quoted_name(one)),
            _ => format!("Expanding {} items", archives.len()),
        };
        self.start_archive_job(label.into(), ArchiveWork::Expand(archives), cx);
    }

    /// File ▸ Compress “x” / Compress N Items.
    pub(super) fn compress_selection(&mut self, cx: &mut Context<Self>) {
        if self.trash_view || self.applications_view {
            return;
        }
        let items = self.selected_paths();
        let label = match items.as_slice() {
            [] => return,
            [one] => format!(
                "Compressing {} to “{}”",
                quoted_name(one),
                rmac_archive::compressed_name(&items)
            ),
            _ => format!(
                "Compressing {} items to “{}”",
                items.len(),
                rmac_archive::compressed_name(&items)
            ),
        };
        self.start_archive_job(label.into(), ArchiveWork::Compress(items), cx);
    }

    fn start_archive_job(
        &mut self,
        label: SharedString,
        work: ArchiveWork,
        cx: &mut Context<Self>,
    ) {
        if self.archive_job.is_some() {
            self.operation_error = Some("Wait for the current archive to finish".into());
            cx.notify();
            return;
        }
        self.menu_at = None;
        self.archive_generation = self.archive_generation.wrapping_add(1);
        let generation = self.archive_generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.archive_job = Some(ArchiveJob {
            generation,
            label,
            progress: rmac_archive::Progress::default(),
            started: Instant::now(),
            visible: false,
            cancel: cancel.clone(),
        });
        cx.notify();

        let (sender, receiver) = async_channel::unbounded::<rmac_archive::Progress>();
        let job = cx.background_executor().spawn(async move {
            let mut report = |progress: rmac_archive::Progress| {
                let _ = sender.try_send(progress);
            };
            work.run(&cancel, &mut report)
        });

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor().timer(SHEET_DELAY).await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if let Some(job) = this
                    .archive_job
                    .as_mut()
                    .filter(|job| job.generation == generation)
                {
                    job.visible = true;
                    cx.notify();
                }
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // The channel closes when the job finishes and drops its sender.
            while let Ok(progress) = receiver.recv().await {
                let updated = this.update(cx, |this: &mut FinderView, cx| {
                    if let Some(job) = this
                        .archive_job
                        .as_mut()
                        .filter(|job| job.generation == generation)
                    {
                        job.progress = progress;
                        if job.visible {
                            cx.notify();
                        }
                    }
                });
                if updated.is_err() {
                    return;
                }
            }
            let failure = job.await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this
                    .archive_job
                    .as_ref()
                    .is_some_and(|job| job.generation == generation)
                {
                    this.archive_job = None;
                }
                if let Some(message) = failure {
                    this.archive_alert = Some(message.into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn cancel_archive_job(&mut self, cx: &mut Context<Self>) {
        if let Some(job) = &self.archive_job {
            job.cancel.store(true, Ordering::Release);
        }
        cx.notify();
    }

    /// The Mac's progress window (404 × 88, measured), drawn as a sheet at
    /// the top of the Files window.
    pub(super) fn render_archive_job(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let job = self.archive_job.as_ref().filter(|job| job.visible)?;
        let fraction = if job.progress.total == 0 {
            0.0
        } else {
            (job.progress.done as f32 / job.progress.total as f32).clamp(0.0, 1.0)
        };
        let status = rmac_archive::progress_line(job.progress, job.started.elapsed());
        Some(
            div()
                .absolute()
                .top(px(60.0))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("archive-progress")
                        .relative()
                        .w(px(404.0))
                        .h(px(88.0))
                        .rounded(px(16.0))
                        .bg(gpui::rgb(0x24212f))
                        .border_1()
                        .border_color(gpui::rgb(0x4f5059))
                        .shadow_lg()
                        .child(div().absolute().left(px(17.0)).top(px(39.0)).child(icon(
                            "icons/file-fill.svg",
                            34.0,
                            gpui::rgb(0xe6e6ea).into(),
                        )))
                        .child(
                            div()
                                .absolute()
                                .left(px(61.0))
                                .top(px(33.0))
                                .w(px(311.0))
                                .h(px(16.0))
                                .text_size(rmac_ui::text_px(13.0))
                                .line_height(px(16.0))
                                .text_color(gpui::rgb(0xdedddf))
                                .truncate()
                                .child(job.label.clone()),
                        )
                        .child(
                            div()
                                .absolute()
                                .left(px(62.0))
                                .top(px(52.5))
                                .w(px(309.0))
                                .h(px(7.0))
                                .rounded(px(3.5))
                                .bg(gpui::rgb(0x363340))
                                .overflow_hidden()
                                .child(
                                    div()
                                        .h_full()
                                        .w(px(309.0 * fraction))
                                        .rounded(px(3.5))
                                        .bg(gpui::rgb(0x3478f6)),
                                ),
                        )
                        .child(
                            div()
                                .id("archive-progress-stop")
                                .absolute()
                                .left(px(378.0))
                                .top(px(48.5))
                                .w(px(15.0))
                                .h(px(15.0))
                                .child(icon(
                                    "icons/quick-look/close.svg",
                                    15.0,
                                    gpui::rgb(0x9d9ba2).into(),
                                ))
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.cancel_archive_job(cx)),
                                ),
                        )
                        .child(
                            div()
                                .absolute()
                                .left(px(61.0))
                                .top(px(64.0))
                                .w(px(311.0))
                                .text_size(rmac_ui::text_px(13.0))
                                .line_height(px(16.0))
                                .text_color(gpui::rgb(0x9c9ba1))
                                .truncate()
                                .child(status),
                        ),
                )
                .into_any_element(),
        )
    }

    /// Archive Utility's alert, in the Files window.
    pub(super) fn render_archive_alert(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let message = self.archive_alert.clone()?;
        Some(
            rmac_ui::alert(
                "Archive Utility",
                message,
                vec![rmac_ui::dialog_button(
                    "archive-alert-ok",
                    "OK",
                    rmac_ui::DialogButtonKind::Primary,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.archive_alert = None;
                    cx.notify();
                }))
                .into_any_element()],
            )
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_labels_follow_finder() {
        assert_eq!(
            compress_menu_label(&[PathBuf::from("/d/notes.txt")]).as_deref(),
            Some("Compress “notes.txt”")
        );
        assert_eq!(
            compress_menu_label(&[PathBuf::from("/d/a"), PathBuf::from("/d/b")]).as_deref(),
            Some("Compress 2 Items")
        );
        assert_eq!(compress_menu_label(&[]), None);
    }
}
