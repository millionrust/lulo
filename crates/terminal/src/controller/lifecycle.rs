//! Terminal window construction, subscriptions, profile startup, and modal admission.

use super::*;

impl TerminalView {
    pub(crate) fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        initial_profile: Option<usize>,
        initial_exec: Option<crate::cli::ExecCommand>,
        // Application ▸ Quit and Keep Windows (TERM-22): a relaunch
        // restoring a saved window. Its first tab takes over every field
        // below that an ordinary launch would otherwise compute from
        // `initial_profile`/`initial_exec`/Settings; the remaining tabs
        // are pushed after construction (see the end of this function).
        initial_restore: Option<crate::session_restore::RestoreWindow>,
    ) -> Self {
        let restoring_tab0 = initial_restore
            .as_ref()
            .and_then(|restore| restore.tabs.first())
            .cloned();
        let (redraw, redraw_rx) = async_channel::bounded(1);
        let (profile, persistence_error) = match load_profile() {
            Ok((profile, legacy_index)) => {
                let migration_error = legacy_index
                    .then(|| save_profile(profile).err())
                    .flatten()
                    .map(|failure| SharedString::from(failure.to_string()));
                (profile, migration_error)
            }
            Err(failure) => (
                profiles::DEFAULT_PROFILE,
                Some(SharedString::from(failure.to_string())),
            ),
        };
        let profile = restoring_tab0
            .as_ref()
            .map(|tab| tab.profile)
            .filter(|index| *index < PROFILES.len())
            .or(initial_profile)
            .unwrap_or(profile);
        let option_as_meta = profiles::load_option_as_meta();
        let font_size = profiles::load_font_size().unwrap_or(FONT_SIZE);
        let (settings, settings_error) = match settings::load() {
            Ok(settings) => (settings, None),
            Err(failure) => (
                crate::settings::Settings::default(),
                Some(SharedString::from(failure.to_string())),
            ),
        };
        let persistence_error = persistence_error.or(settings_error);
        let cols = usize::from(settings.columns);
        let rows = usize::from(settings.rows);
        let scrollback_lines = scrollback_limit_for_tab_count(1);
        let starting_directory = match settings.new_window_directory {
            settings::NewWindowWorkingDirectory::Home => None,
            settings::NewWindowWorkingDirectory::SameWorkingDirectory => {
                crate::working_directory::last_front_directory()
            }
        };
        let starting_directory = restoring_tab0
            .as_ref()
            .and_then(|tab| tab.cwd.clone())
            .or(starting_directory);
        let program = initial_exec.map_or(InitialProgram::Shell, InitialProgram::from);
        let program = restoring_tab0
            .as_ref()
            .map(|tab| match &tab.program {
                Some(program) => InitialProgram::Exec {
                    program: program.clone(),
                    args: tab.args.clone(),
                },
                None => InitialProgram::Shell,
            })
            .unwrap_or(program);
        let session = Session::spawn(
            cols,
            rows,
            scrollback_lines,
            starting_directory,
            redraw.clone(),
            program,
        )
        .unwrap_or_else(|error| Session::failed(cols, rows, scrollback_lines, error));
        if let Some(tab) = &restoring_tab0 {
            session.inject_restored_scrollback(&tab.scrollback);
        }
        // Settings ▸ Window ▸ Size: the OS window itself is resized to the
        // requested cell count using the same cell-measurement fallback
        // `main.rs`'s hard-coded 580×385 used for the 80×24 default — the
        // real cell advance isn't measurable until a window exists, and
        // `resize_to` (called on the first render below) immediately
        // recomputes `cols`/`rows` from whatever size the window actually
        // becomes, exactly as it already does after the user's own resizes.
        //
        // `window.resize` sets GPUI's own window geometry, which still
        // includes Root's client-side shadow/resize-hit margin on Linux
        // (`rmac_ui::CLIENT_FRAME_INSET`) — the same margin `main.rs` already
        // added via `outer_window_size` when it first opened this window.
        // Passing the bare content size here (UIA-02) shrank that margin
        // back out of the window on every launch, so a fresh 80×24 window
        // lost two rows to the top/bottom inset alone (580×361 instead of
        // 580×385, 80×22 instead of 80×24).
        {
            let cell_w = font_size * CELL_RATIO_FALLBACK;
            let line_h = font_size * (LINE_H / FONT_SIZE);
            let width = cols as f32 * cell_w + 2.0 * PAD_X;
            let height = rows as f32 * line_h + TITLE_BAR_HEIGHT + PAD_TOP + PAD_BOTTOM;
            let (outer_width, outer_height) = rmac_ui::outer_window_size(width, height);
            window.resize(gpui::size(px(outer_width), px(outer_height)));
        }

        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        cx.observe(&search, |this, _, cx| {
            let query = bounded_search_query(&this.search.read(cx).value());
            this.tabs[this.active].ui.search_query = query;
            cx.notify();
        })
        .detach();
        // Return in the Find field moves to the next match and Shift-Return
        // to the previous one, as in Terminal's Find bar.
        cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if let InputEvent::PressEnter { shift, .. } = event {
                this.find_step(!*shift, cx);
            }
        })
        .detach();

        cx.bind_keys(super::input::shell_owned_key_bindings());
        // Copy and Paste keep bare Ctrl+C/Ctrl+V free for the shell; see
        // `copy_paste_key_bindings`'s doc comment.
        cx.bind_keys(super::input::copy_paste_key_bindings());
        rmac_ui::bind_keys(
            cx,
            [
                KeyBinding::new("alt-shift-cmd-c", CopyPlainText, Some("Terminal")),
                KeyBinding::new(
                    "ctrl-shift-cmd-c",
                    CopyWithoutBackgroundColour,
                    Some("Terminal"),
                ),
                KeyBinding::new(
                    "ctrl-shift-cmd-/",
                    OpenManPageForSelection,
                    Some("Terminal"),
                ),
                KeyBinding::new(
                    "ctrl-alt-cmd-/",
                    SearchManPageIndexForSelection,
                    Some("Terminal"),
                ),
                KeyBinding::new("ctrl-cmd-v", PasteEscapedText, Some("Terminal")),
                KeyBinding::new("ctrl-shift-cmd-v", PasteEscapedSelection, Some("Terminal")),
                KeyBinding::new(rmac_ui::shortcuts::FIND.keystroke, Find, Some("Terminal")),
                KeyBinding::new(
                    rmac_ui::shortcuts::FIND_NEXT.keystroke,
                    FindNext,
                    Some("Terminal"),
                ),
                KeyBinding::new(
                    rmac_ui::shortcuts::FIND_PREVIOUS.keystroke,
                    FindPrevious,
                    Some("Terminal"),
                ),
                // The same keys while the Find field has focus.
                KeyBinding::new(
                    rmac_ui::shortcuts::FIND_NEXT.keystroke,
                    FindNext,
                    Some("TerminalFind"),
                ),
                KeyBinding::new(
                    rmac_ui::shortcuts::FIND_PREVIOUS.keystroke,
                    FindPrevious,
                    Some("TerminalFind"),
                ),
                KeyBinding::new(
                    rmac_ui::shortcuts::ZOOM_IN.keystroke,
                    ZoomIn,
                    Some("Terminal"),
                ),
                KeyBinding::new(
                    rmac_ui::shortcuts::ZOOM_IN_ALTERNATE.keystroke,
                    ZoomIn,
                    Some("Terminal"),
                ),
                KeyBinding::new(
                    rmac_ui::shortcuts::ZOOM_OUT.keystroke,
                    ZoomOut,
                    Some("Terminal"),
                ),
                KeyBinding::new(
                    rmac_ui::shortcuts::ZOOM_RESET.keystroke,
                    ZoomReset,
                    Some("Terminal"),
                ),
                KeyBinding::new(
                    rmac_ui::shortcuts::SELECT_ALL.keystroke,
                    SelectAll,
                    Some("Terminal"),
                ),
                KeyBinding::new(rmac_ui::shortcuts::CLEAR.keystroke, Clear, Some("Terminal")),
                KeyBinding::new("ctrl-cmd-l", ClearScreen, Some("Terminal")),
                KeyBinding::new("alt-cmd-k", ClearScrollback, Some("Terminal")),
                KeyBinding::new("alt-cmd-o", ToggleOptionAsMeta, Some("Terminal")),
                KeyBinding::new("shift-cmd-t", ShowTabBar, Some("Terminal")),
                KeyBinding::new("cmd-r", AllowMouseReporting, Some("Terminal")),
                KeyBinding::new("cmd-shift-f", HideFindBar, Some("Terminal")),
                KeyBinding::new("cmd-e", UseSelectionForFind, Some("Terminal")),
                KeyBinding::new("cmd-j", JumpToSelection, Some("Terminal")),
                KeyBinding::new("cmd-home", ScrollToTop, Some("Terminal")),
                KeyBinding::new("cmd-end", ScrollToBottom, Some("Terminal")),
                KeyBinding::new("cmd-pageup", PageUp, Some("Terminal")),
                KeyBinding::new("cmd-pagedown", PageDown, Some("Terminal")),
                KeyBinding::new("alt-cmd-pageup", LineUp, Some("Terminal")),
                KeyBinding::new("alt-cmd-pagedown", LineDown, Some("Terminal")),
                KeyBinding::new(
                    rmac_ui::shortcuts::PREVIOUS_MARK.keystroke,
                    PreviousPrompt,
                    Some("Terminal"),
                ),
                KeyBinding::new(
                    rmac_ui::shortcuts::NEXT_MARK.keystroke,
                    NextPrompt,
                    Some("Terminal"),
                ),
                KeyBinding::new("cmd-u", Mark, Some("Terminal")),
                KeyBinding::new("alt-cmd-u", MarkAsBookmark, Some("Terminal")),
                KeyBinding::new("cmd-shift-u", Unmark, Some("Terminal")),
                KeyBinding::new("alt-cmd-up", PreviousBookmark, Some("Terminal")),
                KeyBinding::new("alt-cmd-down", NextBookmark, Some("Terminal")),
                KeyBinding::new("shift-cmd-up", SelectToPreviousMark, Some("Terminal")),
                KeyBinding::new("shift-cmd-down", SelectToNextMark, Some("Terminal")),
                KeyBinding::new(
                    "alt-shift-cmd-up",
                    SelectToPreviousBookmark,
                    Some("Terminal"),
                ),
                KeyBinding::new("alt-shift-cmd-down", SelectToNextBookmark, Some("Terminal")),
                KeyBinding::new(
                    rmac_ui::shortcuts::SELECT_COMMAND_OUTPUT.keystroke,
                    SelectCommandOutput,
                    Some("Terminal"),
                ),
                KeyBinding::new(
                    rmac_ui::shortcuts::NEW_TAB.keystroke,
                    TabBasicDefault,
                    Some("Terminal"),
                ),
                KeyBinding::new(
                    rmac_ui::shortcuts::CLOSE.keystroke,
                    CloseTab,
                    Some("Terminal"),
                ),
                KeyBinding::new("alt-cmd-w", CloseAll, Some("Terminal")),
                KeyBinding::new(
                    rmac_ui::shortcuts::NEXT_TAB.keystroke,
                    NextTab,
                    Some("Terminal"),
                ),
                KeyBinding::new(
                    rmac_ui::shortcuts::PREVIOUS_TAB.keystroke,
                    PrevTab,
                    Some("Terminal"),
                ),
                KeyBinding::new(
                    rmac_ui::shortcuts::CYCLE_PROFILE.keystroke,
                    CycleProfile,
                    Some("Terminal"),
                ),
                // Terminal › Settings… opens the profile list and font size.
                KeyBinding::new(rmac_ui::shortcuts::SETTINGS.keystroke, ShowSettings, None),
                KeyBinding::new(
                    rmac_ui::shortcuts::NEW_WINDOW.keystroke,
                    WindowBasicDefault,
                    Some("Terminal"),
                ),
                KeyBinding::new("alt-cmd-r", ResetTerminal, Some("Terminal")),
                KeyBinding::new("ctrl-alt-cmd-r", HardResetTerminal, Some("Terminal")),
                KeyBinding::new("ctrl-alt-cmd-l", FillScreen, Some("Terminal")),
                KeyBinding::new("cmd-l", ClearToPreviousMark, Some("Terminal")),
                KeyBinding::new("alt-cmd-l", ClearToPreviousBookmark, Some("Terminal")),
                KeyBinding::new("shift-cmd-n", NewCommand, Some("Terminal")),
                KeyBinding::new("shift-cmd-k", NewRemoteConnection, Some("Terminal")),
                KeyBinding::new("cmd-i", ShowInspector, Some("Terminal")),
                KeyBinding::new("shift-cmd-i", EditTitle, Some("Terminal")),
                KeyBinding::new("cmd-d", SplitPane, Some("Terminal")),
                KeyBinding::new("shift-cmd-d", CloseSplitPane, Some("Terminal")),
                KeyBinding::new("escape", CancelNewCommand, Some("TerminalNewCommand")),
                KeyBinding::new("escape", CancelNewRemoteConnection, Some("TerminalRemote")),
                KeyBinding::new("escape", CancelEditTitle, Some("TerminalEditTitle")),
                KeyBinding::new("ctrl-cmd-n", NewWindowWithSameCommand, Some("Terminal")),
                KeyBinding::new("ctrl-cmd-t", NewTabWithSameCommand, Some("Terminal")),
                KeyBinding::new("cmd-s", ExportTextAs, Some("Terminal")),
                KeyBinding::new("shift-cmd-s", ExportSelectedTextAs, Some("Terminal")),
                KeyBinding::new("cmd-p", Print, Some("Terminal")),
                KeyBinding::new("alt-cmd-p", PrintSelection, Some("Terminal")),
                KeyBinding::new("shift-cmd-\\", ShowAllTabs, Some("Terminal")),
                KeyBinding::new("cmd-o", OpenShell, Some("Terminal")),
                KeyBinding::new("escape", CancelOpenShell, Some("TerminalOpenShell")),
                KeyBinding::new("alt-cmd-i", EditBackgroundColour, Some("Terminal")),
                KeyBinding::new(
                    "escape",
                    CancelEditBackgroundColour,
                    Some("TerminalBackgroundColour"),
                ),
                // Application ▸ Quit and Keep Windows: no window-scoped
                // context, like Settings… above, so it reaches the app-level
                // handler even while a different element has focus.
                KeyBinding::new("alt-cmd-q", QuitAndKeepWindows, None),
            ],
        );

        // A close request from outside the window — the Dock's or the menu
        // bar's Quit, ⌘Q, ⌘Tab's Q, or logging out — asks the same question
        // as the red button before it stops running programs, as Terminal
        // does. The view removes the window itself once it may close, so the
        // compositor's request is always declined here.
        let view = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            view.update(cx, |this, cx| {
                this.request_close_window(window, cx);
                if this.pending_close.is_some() {
                    window.activate_window();
                }
            })
            .is_err()
        });

        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let window_active = window.is_window_active();
        cx.observe_window_activation(window, |this, window, cx| {
            this.handle_window_activation(window.is_window_active(), window, cx);
        })
        .detach();

        // PTY/model events wake this task. The bounded channel coalesces output
        // bursts while leaving the application fully asleep when nothing changes.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while redraw_rx.recv().await.is_ok() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();

        let mut view = Self {
            tabs: vec![session],
            redraw,
            active: 0,
            cols,
            rows,
            font_size,
            line_h: font_size * (LINE_H / FONT_SIZE),
            cell_w: measure_cell_w(window, font_size),
            content_origin: (0.0, 0.0),
            focus,
            native_window_title: "Terminal".into(),
            window_active,
            search,
            ime: None,
            selecting: false,
            scroll_accum: 0.0,
            mouse_wheel_x_accum: 0.0,
            mouse_wheel_y_accum: 0.0,
            reported_mouse_press: None,
            last_mouse_report_cell: None,
            hovered_link: None,
            profile,
            tab_profiles: vec![profile],
            show_tab_bar: None,
            allow_mouse_reporting: true,
            picker_open: false,
            option_as_meta,
            cursor_style: settings.cursor_style,
            cursor_blink_enabled: settings.cursor_blink,
            use_bold_fonts: settings.use_bold_fonts,
            bright_bold_text: settings.bright_bold_text,
            display_ansi_colours: settings.display_ansi_colours,
            title_shows_window_size: settings.title_shows_window_size,
            blink_visible: true,
            blink_generation: 0,
            blink_last_input: std::time::Instant::now(),
            blink_parked: false,
            persistence_error,
            operation_error: None,
            pending_close: None,
            pending_paste: None,
            pending_new_command: None,
            pending_remote_connection: None,
            pending_edit_title: None,
            inspector_open: false,
            menu_at: None,
            a11y_cache: None,
            window_generation: NEXT_WINDOW_GENERATION.fetch_add(1, AtomicOrdering::Relaxed),
            automatically_mark_prompt_lines: profiles::load_automatically_mark_prompt_lines(),
            show_marks: false,
            show_all_tabs: false,
            viewing_primary_while_alt_screen: false,
            pending_open_shell: None,
            pending_background_colour: None,
            background_override: profiles::load_background_override(),
        };
        if window_active {
            view.start_cursor_blink(window, cx);
        }
        if let Some(restore) = initial_restore {
            let remaining = restore.tabs.get(1..).unwrap_or(&[]);
            view.apply_additional_restored_tabs(remaining, window, cx);
        }
        register_open_view(cx.entity().downgrade());
        view
    }

    /// Settings ▸ Text ▸ Blink cursor: flips `blink_visible` on a timer
    /// while — and only while — this window is both focused and blink is
    /// on. The loop checks both on every tick and exits the moment either
    /// becomes false, so no timer ever ticks for an unfocused window or one
    /// with blink off; `blink_generation` stops an old loop from fighting a
    /// new one if focus is regained before the old one notices it lost it.
    /// Like the shared text field's insertion point, the cursor also parks
    /// visibly `CURSOR_PARK_AFTER` after the last keystroke: each blink
    /// repaints the whole window, which cost about 15% of a core on a
    /// software-rendered laptop for as long as Terminal stayed focused.
    pub(super) fn start_cursor_blink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.blink_visible = true;
        self.blink_parked = false;
        self.blink_last_input = std::time::Instant::now();
        if !self.cursor_blink_enabled {
            return;
        }
        self.blink_generation = self.blink_generation.wrapping_add(1);
        let generation = self.blink_generation;
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(530))
                .await;
            let should_continue = this
                .update_in(cx, |this, window, cx| {
                    if this.blink_generation != generation
                        || !window.is_window_active()
                        || !this.cursor_blink_enabled
                    {
                        return false;
                    }
                    if cursor_blink_should_park(this.blink_last_input.elapsed()) {
                        this.blink_parked = true;
                        if !this.blink_visible {
                            this.blink_visible = true;
                            cx.notify();
                        }
                        return false;
                    }
                    this.blink_visible = !this.blink_visible;
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !should_continue {
                break;
            }
        })
        .detach();
    }

    /// A keystroke shows the cursor and restarts a parked blink loop.
    pub(super) fn wake_cursor_blink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.blink_last_input = std::time::Instant::now();
        if self.blink_parked && self.window_active {
            self.start_cursor_blink(window, cx);
        }
    }

    pub(super) fn set_profile(&mut self, i: usize, cx: &mut Context<Self>) {
        if i < PROFILES.len() {
            self.profile = i;
            self.tab_profiles[self.active] = i;
            self.picker_open = false;
            self.persistence_error = save_profile(i)
                .err()
                .map(|failure| failure.to_string().into());
            cx.notify();
        }
    }

    pub(super) fn modal_open(&self) -> bool {
        self.pending_close.is_some()
            || self.pending_paste.is_some()
            || self.pending_new_command.is_some()
            || self.pending_remote_connection.is_some()
            || self.pending_edit_title.is_some()
            || self.pending_open_shell.is_some()
            || self.pending_background_colour.is_some()
    }
}

/// How long the cursor keeps blinking after the last keystroke, matching the
/// shared text field's insertion point (`blink_cursor.rs`).
pub(super) const CURSOR_PARK_AFTER: std::time::Duration = std::time::Duration::from_secs(2);

pub(super) fn cursor_blink_should_park(since_last_input: std::time::Duration) -> bool {
    since_last_input >= CURSOR_PARK_AFTER
}
