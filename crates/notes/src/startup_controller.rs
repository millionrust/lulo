use super::*;

pub(super) struct NotesInputs {
    pub(super) search_query: Entity<InputState>,
    pub(super) folder_name_input: Entity<InputState>,
    pub(super) title: Entity<InputState>,
    pub(super) tags: Entity<InputState>,
    pub(super) body: Entity<InputState>,
    pub(super) note_find: Entity<InputState>,
    pub(super) focus: FocusHandle,
}

impl NotesView {
    pub(super) fn initialize_inputs(window: &mut Window, cx: &mut Context<Self>) -> NotesInputs {
        cx.bind_keys([
            KeyBinding::new("cmd-n", ComposeNote, Some("Notes")),
            KeyBinding::new("shift-cmd-n", CreateFolder, Some("Notes")),
            KeyBinding::new("cmd-backspace", TrashOrRestore, Some("Notes")),
            // Plain ⌫, as on the Mac, removes the selected note from the
            // list; a focused text field consumes the key itself first, so
            // this only fires when the list holds the keyboard.
            KeyBinding::new("backspace", DeleteSelectedNote, Some("Notes")),
            KeyBinding::new("cmd-d", DuplicateNote, Some("Notes")),
            // ⌘F is in-note Find; ⌥⌘F is the Mac's Note List Search.
            KeyBinding::new("cmd-f", FindInNote, Some("Notes")),
            KeyBinding::new("cmd-alt-f", FocusSearch, Some("Notes")),
            KeyBinding::new("cmd-g", FindInNoteNext, Some("Notes")),
            KeyBinding::new("shift-cmd-g", FindInNotePrevious, Some("Notes")),
            KeyBinding::new("cmd-shift-e", ExportNotes, Some("Notes")),
            KeyBinding::new(
                rmac_ui::shortcuts::PRINT.keystroke,
                PrintNote,
                Some("Notes"),
            ),
            KeyBinding::new("cmd-shift-l", InsertChecklist, Some("Notes")),
            KeyBinding::new("cmd-b", ToggleBold, Some("Notes")),
            KeyBinding::new("cmd-i", ToggleItalic, Some("Notes")),
            // ⌘W closes the window through the same review as the red
            // button (pending changes, open choosers, running imports).
            KeyBinding::new(
                rmac_ui::shortcuts::CLOSE.keystroke,
                rmac_ui::RequestClose,
                Some("Notes"),
            ),
        ]);

        let search_query = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let folder_name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Folder Name"));
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Title"));
        let tags = cx.new(|cx| InputState::new(window, cx).placeholder("Add Tags"));
        let body = rmac_editor::multiline("Note", window, cx);
        let note_find = cx.new(|cx| InputState::new(window, cx).placeholder("Find in Note"));
        cx.subscribe(&title, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.schedule_current_edit(cx);
            }
        })
        .detach();
        cx.subscribe(&body, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.schedule_current_edit(cx);
            }
        })
        .detach();
        cx.subscribe(&tags, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.schedule_current_edit(cx);
            }
        })
        .detach();
        cx.subscribe(&search_query, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.dispatch_search(cx);
            }
        })
        .detach();
        cx.subscribe(&folder_name_input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.commit_folder_rename(cx);
            }
        })
        .detach();
        cx.subscribe(&note_find, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.note_find_current = 0;
                this.recompute_note_find_matches(cx);
                cx.notify();
            }
        })
        .detach();

        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        NotesInputs {
            search_query,
            folder_name_input,
            title,
            tags,
            body,
            note_find,
            focus,
        }
    }

    pub(super) fn start_workers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let notes_paths = match resolve_notes_paths() {
            Ok(paths) => Some(paths),
            Err(error) => {
                self.message = Some(error.to_string().into());
                None
            }
        };

        if let Some(paths) = notes_paths.as_ref() {
            match NotesWorker::start(paths.clone())
                .map_err(|error| error.to_string())
                .and_then(|worker| {
                    let (client, events) = worker.into_parts();
                    worker_bridge::bridge_worker_events(events, EVENT_CAPACITY)
                        .map(|receiver| (client, receiver))
                        .map_err(|error| format!("Notes could not start its event bridge: {error}"))
                }) {
                Ok((client, receiver)) => {
                    keep_last_edit_on_quit(client.clone(), cx);
                    self.worker = Some(client);
                    cx.spawn_in(window, async move |this, cx| {
                        while let Ok(event) = receiver.recv().await {
                            if this
                                .update_in(cx, |this, window, cx| {
                                    this.apply_worker_event(event, window, cx)
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .detach();
                }
                Err(message) => self.message = Some(message.into()),
            }

            if let Ok((client, receiver)) =
                NotesPreviewWorker::start(paths.data_root().to_path_buf())
                    .map_err(|error| error.to_string())
                    .and_then(|worker| {
                        let (client, events) = worker.into_parts();
                        worker_bridge::bridge_preview_events(
                            events,
                            PREVIEW_EVENT_CAPACITY,
                            worker_bridge::render_preview_image,
                        )
                        .map(|receiver| (client, receiver))
                        .map_err(|error| {
                            format!("Notes could not start its preview bridge: {error}")
                        })
                    })
            {
                self.preview_worker = Some(client);
                cx.spawn_in(window, async move |this, cx| {
                    while let Ok(event) = receiver.recv().await {
                        if this
                            .update_in(cx, |this, _window, cx| this.apply_preview_event(event, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
            }
        }

        match NotesMarkdownPreviewWorker::start()
            .map_err(|error| error.to_string())
            .and_then(|worker| {
                let (client, events) = worker.into_parts();
                worker_bridge::bridge_markdown_preview_events(
                    events,
                    MARKDOWN_PREVIEW_EVENT_CAPACITY,
                )
                .map(|receiver| (client, receiver))
                .map_err(|error| {
                    format!("Notes could not start its Markdown preview bridge: {error}")
                })
            }) {
            Ok((client, receiver)) => {
                self.markdown_preview_worker = Some(client);
                cx.spawn_in(window, async move |this, cx| {
                    while let Ok(event) = receiver.recv().await {
                        if this
                            .update_in(cx, |this, _window, cx| {
                                this.apply_markdown_preview_event(event, cx)
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(message) => {
                if self.message.is_none() {
                    self.message = Some(message.into());
                }
            }
        }

        match NotesSearchWorker::start()
            .map_err(|error| error.to_string())
            .and_then(|worker| {
                let (client, events) = worker.into_parts();
                worker_bridge::bridge_search_events(events, SEARCH_EVENT_CAPACITY)
                    .map(|receiver| (client, receiver))
                    .map_err(|error| format!("Notes could not start its search bridge: {error}"))
            }) {
            Ok((client, receiver)) => {
                self.search_worker = Some(client);
                cx.spawn_in(window, async move |this, cx| {
                    while let Ok(event) = receiver.recv().await {
                        if this
                            .update_in(cx, |this, window, cx| {
                                this.apply_search_event(event, window, cx)
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(message) => {
                if self.message.is_none() {
                    self.message = Some(message.into());
                }
            }
        }
    }
}

/// How long a quitting Notes waits for its repository thread to commit the
/// newest edit before the process exits.
const QUIT_COMMIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(3);

/// However Notes quits — ⌘Q, or SIGTERM because the session is ending — hand
/// the newest typing to the repository worker and wait for it to commit
/// before the process exits. Edits are otherwise committed half a second
/// after the last keystroke, which a shutdown can cut short.
fn keep_last_edit_on_quit(client: NotesWorkerClient, cx: &mut Context<NotesView>) {
    let view = cx.weak_entity();
    gpui::App::on_app_quit(cx, move |cx| {
        if let Some(view) = view.upgrade() {
            view.update(cx, |this, cx| this.schedule_current_edit(cx));
        }
        match client.try_send(WorkerCommand::Shutdown) {
            // Closed: the worker has already stopped.
            Ok(()) | Err(WorkerSendError::Closed) => {}
            Err(error) => eprintln!("Notes could not stop its library cleanly: {error}"),
        }
        if !client.wait_until_stopped(QUIT_COMMIT_LIMIT) {
            eprintln!(
                "Notes quit before its library finished writing; the newest edit may be lost"
            );
        }
        async {}
    })
    .detach();
}
