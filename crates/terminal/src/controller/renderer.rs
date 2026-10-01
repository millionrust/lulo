mod accessibility;
mod chrome;
mod dialogs;
mod grid;
mod interactions;
mod overlays;

use super::*;

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        profiles::set_active(self.profile);
        // Settings ▸ Shell ▸ "When the shell exits": never empties `tabs`
        // (the single-tab case closes the whole *window* instead, leaving
        // `tabs` untouched while GPUI tears it down), so `self.active` stays
        // valid for the rest of this render either way.
        self.auto_close_exited_tabs(window, cx);
        self.resize_to(window);
        // Settings ▸ General ▸ "New windows open with: Same Working
        // Directory" (⌘N) tracks whichever window was frontmost; only a
        // focused window's directory is a candidate.
        if self.window_active {
            if let Some(directory) = self.tabs[self.active].working_directory() {
                crate::working_directory::set_last_front_directory(directory);
            }
        }
        let layout = responsive_layout::terminal_layout(f32::from(
            rmac_ui::window_content_size(window).width,
        ));
        let raw_query = self.search.read(cx).value().to_string();
        let bounded_query = bounded_search_query(&raw_query);
        if bounded_query != raw_query {
            let normalized = bounded_query.clone();
            self.search
                .update(cx, |state, cx| state.set_value(normalized, window, cx));
        }
        self.tabs[self.active].ui.search_query = bounded_query.clone();
        let searching = self.tabs[self.active].ui.search_open;
        let query = if searching {
            bounded_query.to_lowercase()
        } else {
            String::new()
        };
        let rows = self.render_rows(&query);
        let active_title = self.tabs[self.active]
            .tab_title()
            .unwrap_or_else(|| "Terminal".into());
        let native_window_title = rmac_ui::native_window_title(&active_title, "Terminal");
        if self.native_window_title != native_window_title {
            window.set_window_title(&native_window_title);
            self.native_window_title = native_window_title;
        }
        let multi = self.tabs.len() > 1;
        let operation_error_visible = self.operation_error.is_some();
        let terminal_error = self
            .operation_error
            .clone()
            .or_else(|| self.persistence_error.clone());
        let has_terminal_error = terminal_error.is_some();
        let session_status = self.tabs[self.active]
            .status_message()
            .map(SharedString::from);
        let hyperlink_status = session_status
            .is_none()
            .then(|| self.hovered_link.clone())
            .flatten();
        let close_confirmation = self
            .render_close_confirmation(cx)
            .map(|alert| alert.into_any_element());
        let paste_confirmation = self
            .render_paste_confirmation(cx)
            .map(|alert| alert.into_any_element());
        let ime_preedit = (!searching && !self.modal_open())
            .then(|| self.render_ime_preedit())
            .flatten();

        div()
            .size_full()
            .relative()
            .v_flex()
            .bg(hsla(active().bg))
            // The red traffic light dispatches `RequestClose` from
            // whichever element currently holds focus (the grid, the Find
            // field, …), and GPUI bubbles it up the render tree from
            // there. Handling it here on the true root — not on
            // `#terminal-grid`, which is only a sibling of the title bar,
            // Find panel and pickers — keeps the close button working no
            // matter what has focus, mirroring Text Editor/Notes/Finder.
            .on_action(cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| {
                this.request_close_window(window, cx)
            }))
            .child(rmac_ui::title_bar_content(
                self.render_title(active_title, layout.title_max_width),
            ))
            .when(multi, |terminal: Div| {
                terminal.child(self.render_tabs(layout.tab_title_max_width, cx))
            })
            .child(self.render_terminal_body(rows, ime_preedit, window.is_a11y_active(), cx))
            .when(searching, |terminal| {
                terminal.child(self.render_find_panel(layout.find_width, cx))
            })
            // The picker is an absolute overlay — render it LAST so it paints on
            // top of the opaque terminal body instead of behind it.
            .when(self.picker_open, |terminal: Div| {
                terminal.child(self.render_picker(cx))
            })
            // The right-click context menu paints above everything else.
            .when_some(self.menu_at.clone(), |terminal: Div, state| {
                terminal.child(self.render_context_menu(state))
            })
            .when_some(session_status, |terminal, message| {
                terminal.child(self.render_session_status(message, has_terminal_error, cx))
            })
            .when_some(hyperlink_status, |terminal, message| {
                terminal.child(self.render_hyperlink_status(message, has_terminal_error))
            })
            .when_some(terminal_error, |terminal, message| {
                terminal.child(self.render_terminal_error(message, operation_error_visible, cx))
            })
            // Modal reviews remain the final children so no terminal surface
            // can paint over them or receive pointer input.
            .when_some(paste_confirmation, |terminal, alert| terminal.child(alert))
            .when_some(close_confirmation, |terminal, alert| terminal.child(alert))
    }
}

fn hsla(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}
