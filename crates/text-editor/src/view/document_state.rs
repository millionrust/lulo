//! Text Editor document generations, dirty state, recovery, watching, and Recents.

use super::*;

impl EditorView {
    pub(super) fn filename(&self) -> SharedString {
        match &self.path {
            Some(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Untitled".to_string())
                .into(),
            None => "Untitled".into(),
        }
    }

    pub(super) fn file_action_blocked(&self) -> bool {
        self.recovery_loading || self.alert.is_some() || self.print_busy
    }

    fn advance_document_generation(&mut self) {
        self.document_generation = self.document_generation.wrapping_add(1);
        self.current_document_generation
            .store(self.document_generation, Ordering::Release);
    }

    pub(super) fn reset_document_watch(&mut self) {
        self.advance_document_generation();
        self.external_change = None;
        let next_directory = self
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        if self.watched_directory != next_directory {
            if let (Some(watcher), Some(directory)) = (
                self.document_watcher.as_mut(),
                self.watched_directory.take(),
            ) {
                let _ = watcher.unwatch(&directory);
            }
            if let (Some(watcher), Some(directory)) =
                (self.document_watcher.as_mut(), next_directory.as_ref())
            {
                if watcher
                    .watch(directory, notify::RecursiveMode::NonRecursive)
                    .is_ok()
                {
                    self.watched_directory = Some(directory.clone());
                }
            }
        }
        self.document_watch_warning =
            self.path.is_some() && self.watched_directory != next_directory;
    }

    pub(super) fn on_buffer_changed(&mut self, cx: &mut Context<Self>) {
        self.advance_document_generation();
        if self.find_open {
            self.recompute_matches(cx);
        }
        self.refresh_dirty_state(cx);
    }

    pub(super) fn refresh_dirty_state(&mut self, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value().to_string();
        self.dirty = document::has_unsaved_changes(
            &value,
            &self.saved_value,
            self.text_format,
            self.saved_format,
        );
        if self.dirty {
            self.schedule_autosave(cx);
        } else {
            self.clear_recovery(cx);
        }
        cx.notify();
    }

    /// Debounced autosave: each edit bumps a generation token and arms a timer;
    /// only the most recent timer actually writes the recovery file.
    pub(super) fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        let generation = self.recovery_clock.arm();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let Ok(Some((path, record, cleanup_paths))) = this.update(cx, |this, cx| {
                this.recovery_clock
                    .should_write(generation, this.dirty)
                    .then(|| {
                        let content = this.input.read(cx).value().to_string();
                        (
                            this.recovery_path.clone(),
                            recovery::RecoveryRecord::for_document(
                                this.path.as_deref(),
                                this.text_format,
                                content,
                            ),
                            this.recovery_cleanup_paths.clone(),
                        )
                    })
            }) else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    async move {
                        recovery::save(&storage::RealStorage, &path, &record)?;
                        storage::remove_recovery_paths(&storage::RealStorage, &cleanup_paths)
                    }
                })
                .await;
            let stale = this
                .update(cx, |this, cx| {
                    let current = this.recovery_clock.is_current(generation)
                        && this.dirty
                        && this.recovery_path == path;
                    if current {
                        match result {
                            Ok(()) => {
                                this.recovery_cleanup_paths.clear();
                                this.recovery_error = None;
                            }
                            Err(failure) => this.record_recovery_failure(failure, cx),
                        }
                    }
                    !current
                })
                .unwrap_or(false);
            if stale {
                let _ = cx
                    .background_executor()
                    .spawn(async move { storage::remove_recovery(&storage::RealStorage, &path) })
                    .await;
            }
        })
        .detach();
    }

    fn record_recovery_failure(&mut self, _failure: storage::Failure, cx: &mut Context<Self>) {
        self.recovery_error = Some(recovery_failure_message());
        cx.notify();
    }

    pub(super) fn clear_recovery(&mut self, cx: &mut Context<Self>) -> bool {
        self.recovery_clock.invalidate();
        let mut paths = vec![self.recovery_path.clone()];
        paths.extend(self.recovery_cleanup_paths.iter().cloned());
        match storage::remove_recovery_paths(&storage::RealStorage, &paths) {
            Ok(()) => {
                self.recovery_cleanup_paths.clear();
                self.recovery_path = recovery::fresh_record_path(&self.recovery_directory);
                self.recovery_error = None;
                true
            }
            Err(failure) => {
                self.record_recovery_failure(failure, cx);
                false
            }
        }
    }

    pub(super) fn mark_clean(&mut self, value: String, cx: &mut Context<Self>) -> bool {
        self.saved_value = value;
        self.saved_format = self.text_format;
        self.dirty = false;
        self.clear_recovery(cx)
    }

    pub(super) fn record_current_document(&self, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    async move {
                        rmac_recent_documents::Store::from_environment()
                            .and_then(|store| store.record(&path))
                    }
                })
                .await;
            if result.is_err() {
                let _ = this.update(cx, |this, cx| {
                    if this.path.as_deref() == Some(path.as_path()) {
                        this.status_notice =
                            Some("The document is open, but Recents could not be updated.".into());
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }
}
