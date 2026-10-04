use super::*;

pub(super) struct NotesInputs {
    pub(super) search_query: Entity<InputState>,
    pub(super) folder_name_input: Entity<InputState>,
    pub(super) title: Entity<InputState>,
    pub(super) tags: Entity<InputState>,
    pub(super) body: Entity<InputState>,
    pub(super) note_find: Entity<InputState>,
    pub(super) note_replace: Entity<InputState>,
    pub(super) lock_password: Entity<InputState>,
    pub(super) smart_folder_name: Entity<InputState>,
    pub(super) attachment_rename: Entity<InputState>,
    pub(super) focus: FocusHandle,
}

impl NotesView {
    pub(super) fn continue_title_into_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editable = self.is_interactive_ready()
            && !self.markdown_preview_visible
            && self
                .session
                .selected_note()
                .is_some_and(|note| !note.deleted);
        if !editable {
            return;
        }
        let (start, end, text) = {
            let title = self.title.read(cx);
            let range = title.selected_range();
            (range.start, range.end, title.value().to_string())
        };
        let (start, end) = (start.min(text.len()), end.min(text.len()));
        let tail = text[end..].to_string();
        if start < text.len() {
            self.title.update(cx, |title, cx| {
                title.set_selected_range(start..text.len(), cx);
                title.replace("", window, cx);
            });
        }
        let body_empty = self.body.read(cx).value().is_empty();
        self.body.update(cx, |body, cx| {
            body.set_selected_range(0..0, cx);
            if !body_empty || !tail.is_empty() {
                body.insert(format!("{tail}\n"), window, cx);
            }
            body.set_selected_range(0..0, cx);
            body.focus(window, cx);
        });
        self.schedule_current_edit(cx);
        cx.notify();
    }

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
            KeyBinding::new("alt-cmd-w", CloseAll, Some("Notes")),
            KeyBinding::new("alt-cmd-q", QuitAndKeepWindows, Some("Notes")),
            KeyBinding::new("cmd-,", ShowSettings, Some("Notes")),
            KeyBinding::new("cmd-0", FocusMainWindow, Some("Notes")),
            KeyBinding::new("alt-cmd-[", PreviousRecentNote, Some("Notes")),
            KeyBinding::new("alt-cmd-]", NextRecentNote, Some("Notes")),
            // ⌘F is in-note Find; ⌥⌘F is the Mac's Note List Search.
            KeyBinding::new("cmd-f", FindInNote, Some("Notes")),
            KeyBinding::new("shift-cmd-f", FindAndReplace, Some("Notes")),
            KeyBinding::new("cmd-alt-f", FocusSearch, Some("Notes")),
            KeyBinding::new("cmd-g", FindInNoteNext, Some("Notes")),
            KeyBinding::new("shift-cmd-g", FindInNotePrevious, Some("Notes")),
            KeyBinding::new("cmd-e", UseSelectionForFind, Some("Notes")),
            KeyBinding::new("cmd-j", JumpToSelection, Some("Notes")),
            KeyBinding::new("alt-shift-cmd-v", PastePlainText, Some("Notes")),
            KeyBinding::new(
                rmac_ui::shortcuts::PRINT.keystroke,
                PrintNote,
                Some("Notes"),
            ),
            KeyBinding::new("cmd-shift-l", InsertChecklist, Some("Notes")),
            KeyBinding::new("cmd-shift-u", ToggleChecklistDone, Some("Notes")),
            KeyBinding::new("cmd-b", ToggleBold, Some("Notes")),
            KeyBinding::new("cmd-i", ToggleItalic, Some("Notes")),
            KeyBinding::new("shift-cmd-t", SetStyleTitle, Some("Notes")),
            KeyBinding::new("shift-cmd-h", SetStyleHeading, Some("Notes")),
            KeyBinding::new("shift-cmd-j", SetStyleSubheading, Some("Notes")),
            KeyBinding::new("shift-cmd-b", SetStyleBody, Some("Notes")),
            KeyBinding::new("shift-cmd-m", SetStyleMonospaced, Some("Notes")),
            KeyBinding::new("shift-cmd-7", InsertBulletedList, Some("Notes")),
            KeyBinding::new("shift-cmd-8", InsertDashedList, Some("Notes")),
            KeyBinding::new("shift-cmd-9", InsertNumberedList, Some("Notes")),
            KeyBinding::new("cmd-'", InsertBlockQuote, Some("Notes")),
            KeyBinding::new("cmd-k", InsertLink, Some("Notes")),
            KeyBinding::new("cmd-]", IncreaseIndent, Some("Notes")),
            KeyBinding::new("cmd-[", DecreaseIndent, Some("Notes")),
            KeyBinding::new("ctrl-cmd-up", MoveItemUp, Some("Notes")),
            KeyBinding::new("ctrl-cmd-down", MoveItemDown, Some("Notes")),
            KeyBinding::new("alt-cmd-t", InsertTable, Some("Notes")),
            KeyBinding::new("ctrl-cmd-s", ToggleFolders, Some("Notes")),
            KeyBinding::new("cmd-1", ShowListView, Some("Notes")),
            KeyBinding::new("cmd-2", ShowGalleryView, Some("Notes")),
            KeyBinding::new("cmd-3", ToggleAttachmentsBrowser, Some("Notes")),
            KeyBinding::new("alt-cmd-left", CollapseSection, Some("Notes")),
            KeyBinding::new("alt-shift-cmd-left", CollapseAllSections, Some("Notes")),
            KeyBinding::new("alt-cmd-right", ExpandSection, Some("Notes")),
            KeyBinding::new("alt-shift-cmd-right", ExpandAllSections, Some("Notes")),
            KeyBinding::new("shift-cmd-.", ZoomIn, Some("Notes")),
            KeyBinding::new("shift-cmd-,", ZoomOut, Some("Notes")),
            KeyBinding::new("shift-cmd-0", ZoomReset, Some("Notes")),
            KeyBinding::new("cmd-u", ToggleUnderline, Some("Notes")),
            KeyBinding::new("shift-cmd-e", ToggleHighlight, Some("Notes")),
            KeyBinding::new(
                rmac_ui::shortcuts::ZOOM_IN.keystroke,
                FontBigger,
                Some("Notes"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::ZOOM_IN_ALTERNATE.keystroke,
                FontBigger,
                Some("Notes"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::ZOOM_OUT.keystroke,
                FontSmaller,
                Some("Notes"),
            ),
            KeyBinding::new("alt-cmd-c", CopyStyle, Some("Notes")),
            KeyBinding::new("alt-cmd-v", PasteStyle, Some("Notes")),
            KeyBinding::new("cmd-{", AlignLeft, Some("Notes")),
            KeyBinding::new("cmd-|", AlignCentre, Some("Notes")),
            KeyBinding::new("cmd-}", AlignRight, Some("Notes")),
            KeyBinding::new("ctrl-cmd-i", ToggleShowHighlights, Some("Notes")),
            KeyBinding::new("shift-cmd-a", AttachFile, Some("Notes")),
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
        let note_replace = cx.new(|cx| InputState::new(window, cx).placeholder("Replace with"));
        let lock_password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Password")
                .masked(true)
        });
        let smart_folder_name =
            cx.new(|cx| InputState::new(window, cx).placeholder("Smart Folder Name"));
        let attachment_rename =
            cx.new(|cx| InputState::new(window, cx).placeholder("Attachment Name"));
        cx.subscribe(&title, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.schedule_current_edit(cx);
            }
        })
        .detach();
        // Return in the title continues the note on the next line, as the
        // Mac's single text view does: text after the caret moves down into
        // the body and the caret follows it.
        cx.subscribe_in(&title, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(
                event,
                InputEvent::PressEnter {
                    secondary: false,
                    ..
                }
            ) {
                this.continue_title_into_body(window, cx);
            }
        })
        .detach();
        cx.subscribe_in(
            &body,
            window,
            |this, field, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.schedule_current_edit(cx);
                    if this.body_format_editable() {
                        rmac_ui::text_assist::on_text_changed(
                            field,
                            this.text_assist,
                            Some(this.spell_checker.as_ref()
                                as &dyn rmac_ui::text_assist::SpellChecker),
                            window,
                            cx,
                        );
                    }
                }
            },
        )
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
            note_replace,
            lock_password,
            smart_folder_name,
            attachment_rename,
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
