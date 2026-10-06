//! Application ▸ Quit and Keep Windows (TERM-22): this window's half of
//! `crate::session_restore` — capturing its tabs before the app quits, and
//! pushing the tabs a relaunch restores beyond the first one (which
//! `lifecycle::new` already builds the window from).

use crate::session_restore::{bound_scrollback, RestoreTab, RestoreWindow, MAX_RESTORED_TABS};

use super::*;

impl TerminalView {
    /// This window's tabs, each as a [`RestoreTab`]: its working
    /// directory, what it execs (if it is not a plain shell), its colour
    /// profile, and a bounded snapshot of its current buffer text.
    pub(super) fn capture_for_restore(&self) -> RestoreWindow {
        let tabs = self
            .tabs
            .iter()
            .zip(&self.tab_profiles)
            .map(|(session, profile)| {
                let exec = session.exec_origin();
                RestoreTab {
                    cwd: session.working_directory(),
                    program: exec.as_ref().map(|exec| exec.program.clone()),
                    args: exec.map(|exec| exec.args).unwrap_or_default(),
                    profile: *profile,
                    scrollback: session
                        .buffer_text(self.rows, self.cols)
                        .map(|text| bound_scrollback(&text).to_string())
                        .unwrap_or_default(),
                }
            })
            .collect();
        RestoreWindow { tabs }
    }

    /// Pushes every tab in `tabs` (already excluding the one
    /// `lifecycle::new` built this window's first session from), each with
    /// its own working directory, exec and profile, and replays its
    /// scrollback the same way the first tab's was. Stops at
    /// `MAX_RESTORED_TABS`/`MAX_TABS`, whichever is smaller, rather than
    /// failing the whole restore over one oversized saved window.
    pub(super) fn apply_additional_restored_tabs(
        &mut self,
        tabs: &[RestoreTab],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for tab in tabs.iter().take(MAX_RESTORED_TABS) {
            if self.tabs.len() >= MAX_TABS {
                break;
            }
            let next_tab_count = self.tabs.len() + 1;
            let scrollback_lines = scrollback_limit_for_tab_count(next_tab_count);
            if self.apply_scrollback_limit(scrollback_lines).is_err() {
                break;
            }
            let program = match &tab.program {
                Some(program) => InitialProgram::Exec {
                    program: program.clone(),
                    args: tab.args.clone(),
                },
                None => InitialProgram::Shell,
            };
            let session = Session::spawn(
                self.cols.max(MIN_COLS),
                self.rows.max(MIN_ROWS),
                scrollback_lines,
                tab.cwd.clone(),
                self.redraw.clone(),
                program,
            )
            .unwrap_or_else(|error| {
                Session::failed(
                    self.cols.max(MIN_COLS),
                    self.rows.max(MIN_ROWS),
                    scrollback_lines,
                    error,
                )
            });
            session.inject_restored_scrollback(&tab.scrollback);
            self.tabs.push(session);
            self.tab_profiles
                .push(tab.profile.min(PROFILES.len().saturating_sub(1)));
        }
        self.active = 0;
        self.profile = self.tab_profiles[0];
        self.reset_pointer_routing();
        self.sync_search_editor_to_active(window, cx);
        cx.notify();
    }
}
