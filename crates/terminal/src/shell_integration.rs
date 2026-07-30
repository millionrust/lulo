//! Bounded FinalTerm/OSC 133 command-phase state for one terminal session.

use std::sync::{Arc, Mutex};

const MAX_MARKER_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CommandPhase {
    #[default]
    Unknown,
    Prompt,
    Input,
    Running,
    Finished(Option<u8>),
}

#[derive(Clone, Default)]
pub(super) struct SessionShellState {
    phase: Arc<Mutex<CommandPhase>>,
}

impl SessionShellState {
    /// Accept one complete OSC 133 payload after the command separator.
    pub(super) fn set_marker(&self, marker: &str) -> bool {
        let Some(next) = parse_marker(marker) else {
            return false;
        };
        let Ok(mut phase) = self.phase.lock() else {
            return false;
        };
        if *phase == next {
            return false;
        }
        *phase = next;
        true
    }

    /// A report is presentation context only; process control never trusts it.
    pub(super) fn tab_label(&self) -> Option<String> {
        match *self.phase.lock().ok()? {
            CommandPhase::Running => Some("Running".into()),
            CommandPhase::Finished(Some(status)) if status != 0 => Some(format!("Failed {status}")),
            CommandPhase::Unknown
            | CommandPhase::Prompt
            | CommandPhase::Input
            | CommandPhase::Finished(_) => None,
        }
    }
}

fn parse_marker(marker: &str) -> Option<CommandPhase> {
    if marker.is_empty() || marker.len() > MAX_MARKER_BYTES || marker.chars().any(char::is_control)
    {
        return None;
    }
    match marker {
        "A" => Some(CommandPhase::Prompt),
        "B" => Some(CommandPhase::Input),
        "C" => Some(CommandPhase::Running),
        "D" => Some(CommandPhase::Finished(None)),
        _ => marker
            .strip_prefix("D;")?
            .parse::<u8>()
            .ok()
            .map(|status| CommandPhase::Finished(Some(status))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_running_and_failure_without_retaining_command_text() {
        let shell = SessionShellState::default();
        assert!(shell.set_marker("C"));
        assert_eq!(shell.tab_label().as_deref(), Some("Running"));
        assert!(shell.set_marker("D;127"));
        assert_eq!(shell.tab_label().as_deref(), Some("Failed 127"));
        assert!(shell.set_marker("A"));
        assert_eq!(shell.tab_label(), None);
    }

    #[test]
    fn exact_sessions_are_isolated_and_invalid_markers_change_nothing() {
        let first = SessionShellState::default();
        let first_reader = first.clone();
        let second = SessionShellState::default();
        assert!(first_reader.set_marker("D;1"));
        assert_eq!(first.tab_label().as_deref(), Some("Failed 1"));
        assert_eq!(second.tab_label(), None);

        for marker in [
            "",
            "D;-1",
            "D;256",
            "D;secret command",
            "C;unexpected",
            "A\nspoof",
        ] {
            assert!(!first_reader.set_marker(marker), "{marker:?}");
        }
        assert_eq!(first.tab_label().as_deref(), Some("Failed 1"));
    }

    #[test]
    fn successful_or_statusless_completion_has_no_failure_badge() {
        let shell = SessionShellState::default();
        assert!(shell.set_marker("D;0"));
        assert_eq!(shell.tab_label(), None);
        assert!(shell.set_marker("D"));
        assert_eq!(shell.tab_label(), None);
    }
}
