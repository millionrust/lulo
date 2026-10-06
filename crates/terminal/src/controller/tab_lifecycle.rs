//! Terminal tab creation, selection, close review, and resource rebalancing.

use super::*;

impl TerminalView {
    pub(super) fn new_tab_with_profile(
        &mut self,
        profile: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if profile >= PROFILES.len() {
            return;
        }
        let previous_count = self.tabs.len();
        self.new_tab(window, cx);
        if self.tabs.len() > previous_count {
            self.profile = profile;
            self.tab_profiles[self.active] = profile;
            cx.notify();
        }
    }

    /// Terminal ▸ Settings… ▸ General ▸ "Ask before closing": whether a
    /// close (tab or window) should show the terminate-running-processes
    /// review, given the saved policy and whether the closing scope
    /// actually has a foreground job.
    pub(super) fn confirm_close_for_policy(
        policy: settings::AskBeforeClosing,
        has_foreground_job: bool,
    ) -> bool {
        match policy {
            settings::AskBeforeClosing::Never => false,
            settings::AskBeforeClosing::ActiveProcesses => has_foreground_job,
            settings::AskBeforeClosing::Always => true,
        }
    }

    /// Same as `confirm_close_for_policy`, reading the policy fresh from
    /// disk at each close rather than caching it at window creation — like
    /// Shell ▸ "When the shell exits" below, since this only runs on a
    /// user action (not a redraw loop) so a disk read here costs nothing
    /// idle.
    fn should_confirm_close(has_foreground_job: bool) -> bool {
        Self::confirm_close_for_policy(
            settings::load().unwrap_or_default().ask_before_closing,
            has_foreground_job,
        )
    }

    pub(super) fn apply_scrollback_limit(&mut self, limit: usize) -> Result<(), SessionWriteError> {
        // Acquire every authority before mutating any, so one poisoned session
        // cannot leave a partially applied cross-tab budget.
        {
            let mut terms = Vec::with_capacity(self.tabs.len());
            for session in &self.tabs {
                terms.push(session.term.lock().map_err(|_| SessionWriteError::State)?);
            }
            for term in &mut terms {
                // `set_options` updates the primary history even while the
                // alternate screen is active, while preserving the alternate
                // grid's zero-history contract. Updating `grid_mut()` directly
                // would target the wrong grid.
                term.set_options(terminal_config(limit));
            }
        }
        for session in &self.tabs {
            session.set_scrollback_limit(limit);
        }
        Ok(())
    }

    pub(super) fn rebalance_scrollback(&mut self) -> Result<(), SessionWriteError> {
        self.apply_scrollback_limit(scrollback_limit_for_tab_count(self.tabs.len()))
    }

    pub(super) fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_tab_with_program(InitialProgram::Shell, window, cx);
    }

    /// Shell ▸ New Tab with Same Command: a new tab execing exactly what
    /// the active tab is running, when it is running anything in
    /// particular (a plain shell has nothing to repeat, so the menu item
    /// stays disabled for it — see `renderer.rs`).
    pub(super) fn new_tab_with_same_command(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(exec) = self.tabs[self.active].exec_origin() {
            self.new_tab_with_program(InitialProgram::from(exec), window, cx);
        }
    }

    fn new_tab_with_program(
        &mut self,
        program: InitialProgram,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_open() {
            return;
        }
        if self.tabs.len() >= MAX_TABS {
            self.operation_error =
                Some(format!("Terminal supports up to {MAX_TABS} tabs in one window.").into());
            cx.notify();
            return;
        }
        let next_tab_count = self.tabs.len() + 1;
        let scrollback_lines = scrollback_limit_for_tab_count(next_tab_count);
        if let Err(error) = self.apply_scrollback_limit(scrollback_lines) {
            self.operation_error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        if self.window_active {
            let _ = self.report_active_focus(false);
        }
        self.capture_active_search_query(cx);
        let (c, r) = (self.cols.max(MIN_COLS), self.rows.max(MIN_ROWS));
        let starting_directory = self.tabs[self.active].working_directory();
        self.tabs.push(
            Session::spawn(
                c,
                r,
                scrollback_lines,
                starting_directory,
                self.redraw.clone(),
                program,
            )
            .unwrap_or_else(|error| Session::failed(c, r, scrollback_lines, error)),
        );
        self.tab_profiles.push(self.profile);
        self.active = self.tabs.len() - 1;
        self.reset_pointer_routing();
        self.sync_search_editor_to_active(window, cx);
        if self.window_active {
            let _ = self.report_active_focus(true);
        }
        cx.notify();
    }

    pub(super) fn request_close_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_paste.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
            return;
        }
        if self.pending_close.is_some() || index >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            self.request_close_window(window, cx);
            return;
        }
        let session_id = self.tabs[index].id;
        if Self::should_confirm_close(self.tabs[index].has_foreground_job()) {
            self.pending_close = Some(PendingClose::Tab { session_id });
            self.capture_active_search_query(cx);
            self.tabs[self.active].ui.search_open = false;
            self.picker_open = false;
            self.menu_at = None;
            window.focus(&self.focus, cx);
            cx.notify();
            return;
        }
        if let Err(error) = self.tabs[index].terminate() {
            self.operation_error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        self.remove_tab(session_id, window, cx);
    }

    fn remove_tab(&mut self, session_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self
            .tabs
            .iter()
            .position(|session| session.id == session_id)
        else {
            return;
        };
        if self.tabs.len() <= 1 {
            return;
        }
        let previous_active_id = self.tabs[self.active].id;
        self.capture_active_search_query(cx);
        self.tabs.remove(index);
        self.tab_profiles.remove(index);
        if self.active > index {
            self.active -= 1;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        self.profile = self.tab_profiles[self.active];
        self.reset_pointer_routing();
        self.sync_search_editor_to_active(window, cx);
        if self.window_active && self.tabs[self.active].id != previous_active_id {
            let _ = self.report_active_focus(true);
        }
        if let Err(error) = self.rebalance_scrollback() {
            self.operation_error = Some(error.to_string().into());
        }
        cx.notify();
    }

    pub(super) fn request_close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_paste.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
            return;
        }
        if self.pending_close.is_some() {
            return;
        }
        let has_foreground_job = self.tabs.iter().any(Session::has_foreground_job);
        if !Self::should_confirm_close(has_foreground_job) {
            if self.terminate_all().is_err() {
                self.operation_error =
                    Some("Terminal could not terminate every shell safely.".into());
                cx.notify();
                return;
            }
            window.remove_window();
            return;
        }
        self.pending_close = Some(PendingClose::Window);
        self.capture_active_search_query(cx);
        self.tabs[self.active].ui.search_open = false;
        self.picker_open = false;
        self.menu_at = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Settings ▸ Shell ▸ "When the shell exits": close a tab whose shell
    /// exited cleanly (status 0, no signal), if the setting asks for it.
    /// Checked every redraw — including right after a shell exits, since
    /// that transition itself wakes a redraw — so the setting applies
    /// immediately to every open window, unlike the other Settings fields
    /// this crate caches per-window at creation. The cheap in-memory check
    /// (`exited_cleanly`) almost always short-circuits before the rare,
    /// one-time Settings file read.
    pub(super) fn auto_close_exited_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_close.is_some() || !self.tabs.iter().any(Session::exited_cleanly) {
            return;
        }
        if settings::load().unwrap_or_default().when_shell_exits
            != settings::ShellExitBehavior::CloseIfCleanExit
        {
            return;
        }
        let mut index = self.tabs.len();
        while index > 0 {
            index -= 1;
            if self.tabs[index].exited_cleanly() {
                self.request_close_tab(index, window, cx);
                if self.pending_close.is_some() || self.tabs.is_empty() {
                    return;
                }
            }
        }
    }

    fn terminate_all(&mut self) -> Result<(), SessionControlError> {
        let mut failed = false;
        for session in &mut self.tabs {
            failed |= session.terminate().is_err();
        }
        if failed {
            Err(SessionControlError)
        } else {
            Ok(())
        }
    }

    pub(super) fn cancel_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_close = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn confirm_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_close.take() else {
            return;
        };
        match pending {
            PendingClose::Tab { session_id } => {
                let result = self
                    .tabs
                    .iter_mut()
                    .find(|session| session.id == session_id)
                    .map(Session::terminate)
                    .unwrap_or(Ok(()));
                if let Err(error) = result {
                    self.operation_error = Some(error.to_string().into());
                    window.focus(&self.focus, cx);
                    cx.notify();
                    return;
                }
                self.remove_tab(session_id, window, cx);
                window.focus(&self.focus, cx);
            }
            PendingClose::Window => {
                if self.terminate_all().is_err() {
                    self.operation_error =
                        Some("Terminal could not terminate every shell safely.".into());
                    window.focus(&self.focus, cx);
                    cx.notify();
                    return;
                }
                window.remove_window();
            }
        }
    }

    pub(super) fn select_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() || index >= self.tabs.len() || index == self.active {
            return;
        }
        if self.window_active {
            let _ = self.report_active_focus(false);
        }
        self.capture_active_search_query(cx);
        self.active = index;
        self.profile = self.tab_profiles[index];
        self.reset_pointer_routing();
        self.sync_search_editor_to_active(window, cx);
        if self.window_active {
            let _ = self.report_active_focus(true);
        }
        cx.notify();
    }

    pub(super) fn next_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() > 1 {
            let next = (self.active + 1) % self.tabs.len();
            self.select_tab(next, window, cx);
        }
    }

    pub(super) fn prev_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() > 1 {
            let previous = (self.active + self.tabs.len() - 1) % self.tabs.len();
            self.select_tab(previous, window, cx);
        }
    }
}
