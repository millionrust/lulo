//! Terminal window construction, subscriptions, profile startup, and modal admission.

use super::*;

impl TerminalView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (redraw, redraw_rx) = async_channel::bounded(1);
        let scrollback_lines = scrollback_limit_for_tab_count(1);
        let session = Session::spawn(COLS, ROWS, scrollback_lines, None, redraw.clone())
            .unwrap_or_else(|error| Session::failed(COLS, ROWS, scrollback_lines, error));

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

        cx.bind_keys([
            KeyBinding::new(rmac_ui::shortcuts::COPY.keystroke, Copy, Some("Terminal")),
            KeyBinding::new(rmac_ui::shortcuts::PASTE.keystroke, Paste, Some("Terminal")),
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
            KeyBinding::new(
                rmac_ui::shortcuts::SELECT_COMMAND_OUTPUT.keystroke,
                SelectCommandOutput,
                Some("Terminal"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::NEW_TAB.keystroke,
                NewTab,
                Some("Terminal"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::CLOSE.keystroke,
                CloseTab,
                Some("Terminal"),
            ),
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
            KeyBinding::new(
                rmac_ui::shortcuts::SETTINGS.keystroke,
                ShowSettings,
                Some("Terminal"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::NEW_WINDOW.keystroke,
                NewWindow,
                Some("Terminal"),
            ),
            KeyBinding::new("alt-cmd-r", ResetTerminal, Some("Terminal")),
            KeyBinding::new("ctrl-alt-cmd-r", HardResetTerminal, Some("Terminal")),
        ]);

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
        let option_as_meta = profiles::load_option_as_meta();
        let font_size = profiles::load_font_size().unwrap_or(FONT_SIZE);

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

        Self {
            tabs: vec![session],
            redraw,
            active: 0,
            cols: COLS,
            rows: ROWS,
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
            picker_open: false,
            option_as_meta,
            persistence_error,
            operation_error: None,
            pending_close: None,
            pending_paste: None,
            menu_at: None,
            a11y_cache: None,
        }
    }

    pub(super) fn set_profile(&mut self, i: usize, cx: &mut Context<Self>) {
        if i < PROFILES.len() {
            self.profile = i;
            self.picker_open = false;
            self.persistence_error = save_profile(i)
                .err()
                .map(|failure| failure.to_string().into());
            cx.notify();
        }
    }

    pub(super) fn modal_open(&self) -> bool {
        self.pending_close.is_some() || self.pending_paste.is_some()
    }
}
