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

        cx.bind_keys([
            KeyBinding::new(rmac_ui::shortcuts::COPY.keystroke, Copy, Some("Terminal")),
            KeyBinding::new(rmac_ui::shortcuts::PASTE.keystroke, Paste, Some("Terminal")),
            KeyBinding::new(rmac_ui::shortcuts::FIND.keystroke, Find, Some("Terminal")),
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
        ]);

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
            font_size: FONT_SIZE,
            line_h: LINE_H,
            cell_w: measure_cell_w(window, FONT_SIZE),
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
            persistence_error,
            operation_error: None,
            pending_close: None,
            pending_paste: None,
            menu_at: None,
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
