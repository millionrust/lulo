use super::*;

impl NotesView {
    pub(super) fn apply_search_event(
        &mut self,
        event: SearchWorkerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(&event, SearchWorkerEvent::Stopped { .. }) {
            self.search.cancel();
            let unexpected = !self.closing && !self.search_shutdown_requested;
            self.search_shutdown_requested = true;
            self.search_worker = None;
            if unexpected {
                self.message = Some("Notes search stopped unexpectedly".into());
            }
            cx.notify();
            return;
        }
        if event.project(&mut self.search) {
            if self.search.state() == SearchState::Results && self.is_interactive_ready() {
                if let Some(note_id) = self.search.selected() {
                    let previous = self.session.selected_note_id();
                    self.session
                        .select_folder(rmac_notes_runtime::FolderSelection::All);
                    if self.session.select_note(note_id)
                        && (previous != Some(note_id) || self.latest_local_generation.is_none())
                    {
                        self.sync_editor(window, cx);
                    }
                }
            }
            cx.notify();
        }
    }

    /// AT-SPI's `SetValue`/`ReplaceSelectedText` for the search field, wired
    /// the same way `crates/launcher-app` wires Spotlight's search field:
    /// both actions replace the whole query, then run the same
    /// search-dispatch path a keystroke takes (`set_value` itself emits no
    /// change event).
    pub(super) fn assistive_search_listener(
        &self,
        cx: &Context<Self>,
    ) -> impl FnMut(Option<&accesskit::ActionData>, &mut Window, &mut gpui::App) + 'static {
        let view = cx.entity();
        move |data, window, cx| {
            let Some(accesskit::ActionData::Value(text)) = data else {
                return;
            };
            let text = text.to_string();
            view.update(cx, |this, cx| {
                this.search_query
                    .update(cx, |state, cx| state.set_value(text, window, cx));
                this.dispatch_search(cx);
            });
        }
    }

    pub(super) fn dispatch_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search_query.read(cx).value().to_string();
        if query.trim().is_empty() {
            self.search.cancel();
            cx.notify();
            return;
        }
        let Some(snapshot) = self.session.snapshot().cloned() else {
            self.search.cancel();
            return;
        };
        let Some(worker) = self.search_worker.clone() else {
            self.search.cancel();
            self.message = Some("Notes search is unavailable".into());
            cx.notify();
            return;
        };
        let request = match self
            .search
            .begin(query, MAX_SEARCH_RESULTS, snapshot.revision)
        {
            Ok(Some(request)) => request,
            Ok(None) => {
                cx.notify();
                return;
            }
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if let Err(error) = worker.try_run(snapshot, request) {
            self.search.cancel();
            self.message = Some(error.to_string().into());
        }
        cx.notify();
    }

    pub(super) fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.folder_dialog.is_some()
            || self.purge_dialog.is_some()
            || self.move_dialog.is_some()
            || self.attachment_dialog.is_some()
            || self.export_dialog.is_some()
            || self.attachment_chooser_open
            || self.note_import_chooser_open
            || self.note_import_request_id.is_some()
            || self.markdown_import_review.is_some()
            || self.markdown_import_action_request_id.is_some()
            || self.export_chooser_open
            || self.export_request_id.is_some()
            || self.bundle_chooser_open
            || self.bundle_review_request_id.is_some()
            || self.bundle_review.is_some()
            || self.bundle_action_request_id.is_some()
            || self.bundle_import_completion.is_some()
            || self.attachment_action_pending()
        {
            return;
        }
        self.search_query
            .update(cx, |state, cx| state.focus(window, cx));
    }

    pub(super) fn select_search_result(
        &mut self,
        note_id: NoteId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() || !self.search.select(note_id) {
            return;
        }
        let previous = self.session.selected_note_id();
        self.session
            .select_folder(rmac_notes_runtime::FolderSelection::All);
        if self.session.select_note(note_id)
            && (previous != Some(note_id) || self.latest_local_generation.is_none())
        {
            self.sync_editor(window, cx);
        }
        cx.notify();
    }

    pub(super) fn request_search_shutdown(&mut self, cx: &mut Context<Self>) -> bool {
        if self.search_shutdown_requested {
            return true;
        }
        self.search.cancel();
        let result = self
            .search_worker
            .as_ref()
            .ok_or(SearchWorkerSendError::Closed)
            .and_then(NotesSearchWorkerClient::try_shutdown);
        match result {
            Ok(()) | Err(SearchWorkerSendError::Closed) => {
                self.search_shutdown_requested = true;
                true
            }
            Err(error) => {
                self.message =
                    Some(format!("Notes search is still finishing: {error}. Try again.").into());
                cx.notify();
                false
            }
        }
    }
}
