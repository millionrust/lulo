//! Text Editor window construction, subscriptions, recovery startup, and file watching.

use super::*;

static NEXT_WINDOW_GENERATION: AtomicU64 = AtomicU64::new(1);

impl EditorView {
    pub(super) fn new_with_path(
        initial_path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = crate::settings::current();
        let input = rmac_editor::multiline("", window, cx);
        let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        let replace_input = cx.new(|cx| InputState::new(window, cx).placeholder("Replace with"));
        let save_name_input = cx.new(|cx| InputState::new(window, cx).default_value("Untitled"));
        let save_goto_input = cx.new(|cx| InputState::new(window, cx).placeholder("Go to Folder"));

        // TextEdit-style untitled numbering: only a window that opens with
        // no path (never one about to load a file) claims a number, freed
        // by `release_untitled_slot` once it gets a path or closes.
        let untitled_slot = initial_path.is_none().then(Self::claim_untitled_slot);
        if let Some(slot) = untitled_slot {
            cx.on_release(move |_, _cx| Self::release_untitled_slot_number(slot))
                .detach();
        }
        let (document_events, document_event_rx) = async_channel::bounded(4);
        let document_watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                let event = match result {
                    Ok(event) if matches!(event.kind, notify::EventKind::Access(_)) => return,
                    Ok(_) => DocumentWatchEvent::Changed,
                    Err(_) => DocumentWatchEvent::Unavailable,
                };
                let _ = document_events.try_send(event);
            })
            .ok();

        // Dirty tracking + live match refresh + autosave on every edit.
        let sub_main = cx.subscribe(&input, |this, _input, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                this.on_buffer_changed(cx);
            }
        });
        // Live match recompute as the query is edited; Return submits the
        // search (Shift+Return repeats backward), the way TextEdit's own
        // Find field does — `find_input` is single-line, so GPUI's
        // `InputState` never inserts a newline for Enter, only emits this.
        let sub_find = cx.subscribe(&find_input, |this, _input, ev: &InputEvent, cx| match ev {
            InputEvent::Change => {
                this.current = 0;
                this.recompute_matches(cx);
                cx.notify();
            }
            InputEvent::PressEnter { shift, .. } => {
                if *shift {
                    this.find_prev(cx);
                } else {
                    this.submit_find(cx);
                }
            }
            _ => {}
        });
        let sub_save_goto = cx.subscribe_in(
            &save_goto_input,
            window,
            |this, _input, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.commit_save_goto(window, cx);
                }
            },
        );

        cx.bind_keys([
            KeyBinding::new("cmd-shift-g", SaveGoToFolder, Some("Input")),
            KeyBinding::new(rmac_ui::shortcuts::NEW.keystroke, NewFile, Some(CTX)),
            KeyBinding::new(rmac_ui::shortcuts::OPEN.keystroke, OpenFile, Some(CTX)),
            KeyBinding::new("cmd-,", ShowSettings, Some(CTX)),
            KeyBinding::new(rmac_ui::shortcuts::SAVE.keystroke, SaveFile, Some(CTX)),
            KeyBinding::new(rmac_ui::shortcuts::SAVE_AS.keystroke, SaveFileAs, Some(CTX)),
            KeyBinding::new(
                rmac_ui::shortcuts::DUPLICATE_DOCUMENT.keystroke,
                DuplicateDocument,
                Some(CTX),
            ),
            KeyBinding::new(rmac_ui::shortcuts::FIND.keystroke, ToggleFind, Some(CTX)),
            KeyBinding::new(
                rmac_ui::shortcuts::REPLACE.keystroke,
                ToggleReplace,
                Some(CTX),
            ),
            KeyBinding::new(rmac_ui::shortcuts::FIND_NEXT.keystroke, FindNext, Some(CTX)),
            KeyBinding::new(
                rmac_ui::shortcuts::FIND_PREVIOUS.keystroke,
                FindPrev,
                Some(CTX),
            ),
            KeyBinding::new("cmd-e", UseSelectionForFind, Some(CTX)),
            KeyBinding::new("cmd-j", JumpToSelection, Some(CTX)),
            KeyBinding::new("cmd-0", ActualSize, Some(CTX)),
            KeyBinding::new("cmd-shift-.", IncreaseFont, Some(CTX)),
            KeyBinding::new("cmd-shift-,", DecreaseFont, Some(CTX)),
            KeyBinding::new(rmac_ui::shortcuts::ESCAPE.keystroke, CloseBar, Some(CTX)),
            KeyBinding::new("cmd-.", CloseBar, Some(CTX)),
            KeyBinding::new(
                rmac_ui::shortcuts::ZOOM_IN.keystroke,
                IncreaseFont,
                Some(CTX),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::ZOOM_IN_ALTERNATE.keystroke,
                IncreaseFont,
                Some(CTX),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::ZOOM_OUT.keystroke,
                DecreaseFont,
                Some(CTX),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::TOGGLE_MONOSPACE.keystroke,
                ToggleMono,
                Some(CTX),
            ),
            KeyBinding::new("cmd-shift-w", ToggleWrapToPage, Some(CTX)),
            KeyBinding::new(rmac_ui::shortcuts::CLOSE.keystroke, CloseWindow, Some(CTX)),
        ]);
        #[cfg(target_os = "linux")]
        cx.bind_keys([KeyBinding::new(
            rmac_ui::shortcuts::PRINT.keystroke,
            PrintFile,
            Some(CTX),
        )]);

        // A close request from outside the window (the Dock's or the menu
        // bar's Quit, ⌘Tab's Q, or logging out) takes the same unsaved-changes
        // path as ⌘W, as on the Mac (TE-18): a clean document closes, a
        // dirty saved one autosaves and closes, and a dirty Untitled one
        // comes forward with the Save sheet (Delete / Cancel / Save) — or
        // with the alert or save already in progress. The guard removes the
        // window itself when it may close, so the compositor's request is
        // always declined here.
        let view = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            view.update(cx, |this, cx| {
                if this.dirty || this.file_busy || this.file_action_blocked() {
                    window.activate_window();
                }
                this.guarded(Pending::Close, window, cx);
            })
            .is_err()
        });

        // However the session ends — a shutdown from Terminal, the power
        // button, logind, SIGTERM — write the newest text to this window's
        // recovery draft first, so the next launch offers it back instead of
        // losing up to the two seconds the autosave waits.
        rmac_ui::session::preserve_on_session_end(cx, |this, cx| this.preserve_recovery_now(cx));

        // Recovery discovery can inspect bounded records totaling up to 128
        // MiB. Present the first frame immediately and keep the document gated
        // until the background result establishes this window's recovery
        // identity and any required Restore/Discard decision.
        cx.spawn_in(window, async move |this, cx| {
            let recovery = cx
                .background_executor()
                .spawn(async { startup_recovery() })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.recovery_directory = recovery.directory;
                this.recovery_path = recovery.active_path;
                this.recovery_cleanup_paths = recovery.cleanup_paths;
                this.recovery_loading = false;
                this.recovery_error = recovery.warning.then(recovery_failure_message);
                this.alert = recovery.prompt.map(ActiveAlert::Recover);
                if this.alert.is_none() {
                    // TextEdit opens with the insertion point live in the
                    // text: focus the (now enabled) body so the caret shows
                    // and typing lands without a click.
                    this.input.update(cx, |state, cx| state.focus(window, cx));
                    if let Some(path) = this.pending_startup_path.take() {
                        this.load_document_path(path, "The file could not be opened.", window, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();

        // Native events are hints only. Coalesce bursts from atomic rename and
        // metadata activity, then compare the complete bounded file bytes with
        // the exact revision retained at open/last-save.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = document_event_rx.recv().await {
                if matches!(event, DocumentWatchEvent::Unavailable) {
                    if this
                        .update(cx, |this, cx| {
                            this.document_watch_warning = this.path.is_some();
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                    continue;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let mut unavailable = false;
                while let Ok(event) = document_event_rx.try_recv() {
                    unavailable |= matches!(event, DocumentWatchEvent::Unavailable);
                }
                if unavailable
                    && this
                        .update(cx, |this, cx| {
                            this.document_watch_warning = this.path.is_some();
                            cx.notify();
                        })
                        .is_err()
                {
                    break;
                }
                let snapshot = this
                    .update(cx, |this, _| {
                        this.path
                            .clone()
                            .zip(this.saved_bytes.clone())
                            .map(|(path, expected)| (path, expected, this.document_generation))
                    })
                    .ok()
                    .flatten();
                let Some((path, expected, generation)) = snapshot else {
                    continue;
                };
                let checked_path = path.clone();
                let state = cx
                    .background_executor()
                    .spawn(async move { inspect_external_revision(&checked_path, &expected) })
                    .await;
                if this
                    .update(cx, |this, cx| {
                        if this.document_generation == generation
                            && this.path.as_deref() == Some(path.as_path())
                        {
                            this.external_change = state;
                            if !unavailable {
                                this.document_watch_warning = false;
                            }
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let recovery_directory = std::env::temp_dir().join("rmac-text-editor-recovery-pending");
        let recovery_path = recovery::fresh_record_path(&recovery_directory);

        Self {
            alert: None,
            input,
            path: None,
            untitled_slot,
            saved_bytes: None,
            text_format: document::TextFormat {
                encoding: settings.default_encoding,
                ..document::TextFormat::default()
            },
            saved_format: document::TextFormat {
                encoding: settings.default_encoding,
                ..document::TextFormat::default()
            },
            saved_text: Rope::new(),
            text_revision: 0,
            saved_revision: 0,
            accessible_value: None,
            long_lines: None,
            dirty: false,
            file_busy: false,
            print_busy: false,
            find_open: false,
            replace_mode: false,
            find_input,
            replace_input,
            save_name_input,
            save_goto_input,
            save_goto_open: false,
            save_goto_busy: false,
            save_goto_error: false,
            save_custom_folder: None,
            save_location: SaveLocation::default(),
            matches: Vec::new(),
            current: 0,
            // TextEdit's plain-text default: Menlo 11 (JetBrains Mono here).
            mono: true,
            font_size: f32::from(settings.font_size),
            wrap_to_page: settings.wrap_to_page,
            page_width_chars: settings.width_chars,
            rtf_runs: None,
            focus: cx.focus_handle(),
            native_window_title: "Text Editor".into(),
            recovery_directory,
            recovery_path,
            recovery_cleanup_paths: Vec::new(),
            recovery_clock: RecoveryClock::default(),
            recovery_writer: recovery::RecoveryWriter::default(),
            recovery_loading: true,
            closing: false,
            recovery_error: None,
            status_notice: None,
            window_generation: NEXT_WINDOW_GENERATION
                .fetch_add(1, Ordering::Relaxed)
                .max(1),
            document_generation: 0,
            current_document_generation: Arc::new(AtomicU64::new(0)),
            external_change: None,
            document_watch_warning: false,
            watched_directory: None,
            document_watcher,
            pending_startup_path: initial_path,
            _subscriptions: vec![sub_main, sub_find, sub_save_goto],
        }
    }
}
