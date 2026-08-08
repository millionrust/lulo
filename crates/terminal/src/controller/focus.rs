//! Terminal window and xterm focus-report orchestration.

use super::*;

impl TerminalView {
    fn report_focus_for_session(
        &mut self,
        index: usize,
        focused: bool,
    ) -> Result<(), SessionWriteError> {
        if !self.tabs[index].accepts_input() {
            return Ok(());
        }
        let report = {
            let terminal = self.tabs[index]
                .term
                .lock()
                .map_err(|_| SessionWriteError::State)?;
            focus_report(*terminal.mode(), focused)
        };
        let Some(report) = report else {
            return Ok(());
        };
        self.tabs[index].write(report)
    }

    /// Report one truthful focus transition without repainting the ordinary
    /// success path. Returns whether visible failure state changed.
    pub(super) fn report_active_focus(&mut self, focused: bool) -> bool {
        let was_live = self.tabs[self.active].accepts_input();
        let result = self.report_focus_for_session(self.active, focused);
        if matches!(result, Err(SessionWriteError::State)) {
            self.operation_error = Some(SessionWriteError::State.to_string().into());
        }
        was_live != self.tabs[self.active].accepts_input()
            || matches!(result, Err(SessionWriteError::State))
    }

    pub(super) fn handle_window_activation(
        &mut self,
        active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.window_active == active {
            return;
        }
        self.window_active = active;
        let menu_closed = !active && rmac_ui::ContextMenuState::dismiss(&mut self.menu_at, window);
        if self.report_active_focus(active) || menu_closed {
            cx.notify();
        }
    }
}
