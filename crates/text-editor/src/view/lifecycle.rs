//! Text Editor window construction, subscriptions, recovery startup, and file watching.

use super::*;

static NEXT_WINDOW_GENERATION: AtomicU64 = AtomicU64::new(1);

impl EditorView {
    pub(super) fn new_with_path(
        initial_path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = rmac_editor::multiline("", window, cx);
        let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        let replace_input = cx.new(|cx| InputState::new(window, cx).placeholder("Replace with"));
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
        // Re-render the parent whenever the buffer notifies — InputEvent has no
        // cursor-move variant, but `observe` fires on every `notify()` the input
        // makes (including caret movement), keeping the line:col status live.
        cx.observe(&input, |_, _, cx| cx.notify()).detach();

        // Live match recompute as the query is edited.
        let sub_find = cx.subscribe(&find_input, |this, _input, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                this.current = 0;
                this.recompute_matches(cx);
                cx.notify();
            }
        });

        cx.bind_keys([
            KeyBinding::new(rmac_ui::shortcuts::NEW.keystroke, NewFile, Some(CTX)),
            KeyBinding::new(rmac_ui::shortcuts::OPEN.keystroke, OpenFile, Some(CTX)),
            KeyBinding::new(rmac_ui::shortcuts::SAVE.keystroke, SaveFile, Some(CTX)),
            KeyBinding::new(rmac_ui::shortcuts::SAVE_AS.keystroke, SaveFileAs, Some(CTX)),
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
            KeyBinding::new(rmac_ui::shortcuts::ESCAPE.keystroke, CloseBar, Some(CTX)),
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
            KeyBinding::new(rmac_ui::shortcuts::CLOSE.keystroke, CloseWindow, Some(CTX)),
        ]);
        #[cfg(target_os = "linux")]
        cx.bind_keys([KeyBinding::new(
            rmac_ui::shortcuts::PRINT.keystroke,
            PrintFile,
            Some(CTX),
        )]);

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
            saved_bytes: None,
            text_format: document::TextFormat::default(),
            saved_format: document::TextFormat::default(),
            saved_value: String::new(),
            dirty: false,
            file_busy: false,
            print_busy: false,
            find_open: false,
            replace_mode: false,
            find_input,
            replace_input,
            matches: Vec::new(),
            current: 0,
            // TextEdit's plain-text default: Menlo 11 (JetBrains Mono here).
            mono: true,
            font_size: 11.0,
            rtf_runs: None,
            focus: cx.focus_handle(),
            native_window_title: "Text Editor".into(),
            recovery_directory,
            recovery_path,
            recovery_cleanup_paths: Vec::new(),
            recovery_clock: RecoveryClock::default(),
            recovery_loading: true,
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
            _subscriptions: vec![sub_main, sub_find],
        }
    }
}
