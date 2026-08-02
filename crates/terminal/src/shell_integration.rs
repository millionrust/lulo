//! Bounded FinalTerm/OSC 133 command-phase and prompt-mark state for one session.

use std::sync::{Arc, Mutex};

const MAX_MARKER_BYTES: usize = 32;
const MAX_PROMPT_MARKS: usize = 512;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CommandPhase {
    #[default]
    Unknown,
    Prompt,
    Input,
    Running,
    Finished(Option<u8>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PromptDirection {
    Previous,
    Next,
}

#[derive(Debug, Default)]
struct ShellState {
    phase: CommandPhase,
    /// Retained-grid line coordinates only; no prompt or command text is stored.
    prompt_lines: Vec<usize>,
}

#[derive(Clone, Default)]
pub(super) struct SessionShellState {
    state: Arc<Mutex<ShellState>>,
}

impl SessionShellState {
    /// Accept one complete OSC 133 payload after the command separator. A
    /// prompt coordinate is supplied only when the parser is on the primary
    /// grid and its retained history has not reached the eviction boundary.
    pub(super) fn set_marker(&self, marker: &str, prompt_line: Option<usize>) -> bool {
        let Some(next) = parse_marker(marker) else {
            return false;
        };
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let phase_changed = state.phase != next;
        state.phase = next;

        let mut prompt_added = false;
        if next == CommandPhase::Prompt {
            if let Some(line) = prompt_line.filter(|line| !state.prompt_lines.contains(line)) {
                if state.prompt_lines.len() == MAX_PROMPT_MARKS {
                    state.prompt_lines.remove(0);
                }
                state.prompt_lines.push(line);
                prompt_added = true;
            }
        }
        phase_changed || prompt_added
    }

    /// A report is presentation context only; process control never trusts it.
    pub(super) fn tab_label(&self) -> Option<String> {
        match self.state.lock().ok()?.phase {
            CommandPhase::Running => Some("Running".into()),
            CommandPhase::Finished(Some(status)) if status != 0 => Some(format!("Failed {status}")),
            CommandPhase::Unknown
            | CommandPhase::Prompt
            | CommandPhase::Input
            | CommandPhase::Finished(_) => None,
        }
    }

    pub(super) fn clear_prompt_marks(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.prompt_lines.clear();
        }
    }

    /// Resolve a prompt to an Alacritty display offset without retaining any
    /// shell text. Marks fail closed at the history eviction boundary because
    /// retained-grid coordinates are no longer stable after that point.
    pub(super) fn prompt_offset(
        &self,
        direction: PromptDirection,
        history_size: usize,
        display_offset: usize,
        history_limit: usize,
    ) -> Option<usize> {
        if history_limit == 0 || history_size >= history_limit {
            return None;
        }
        let viewport_top = history_size.saturating_sub(display_offset);
        let state = self.state.lock().ok()?;
        let target = match direction {
            PromptDirection::Previous => state
                .prompt_lines
                .iter()
                .copied()
                .filter(|line| *line < viewport_top)
                .max(),
            PromptDirection::Next => state
                .prompt_lines
                .iter()
                .copied()
                .filter(|line| *line > viewport_top)
                .min(),
        }?;
        Some(history_size.saturating_sub(target).min(history_size))
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
        assert!(shell.set_marker("C", None));
        assert_eq!(shell.tab_label().as_deref(), Some("Running"));
        assert!(shell.set_marker("D;127", None));
        assert_eq!(shell.tab_label().as_deref(), Some("Failed 127"));
        assert!(shell.set_marker("A", Some(4)));
        assert_eq!(shell.tab_label(), None);
    }

    #[test]
    fn exact_sessions_are_isolated_and_invalid_markers_change_nothing() {
        let first = SessionShellState::default();
        let first_reader = first.clone();
        let second = SessionShellState::default();
        assert!(first_reader.set_marker("D;1", None));
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
            assert!(!first_reader.set_marker(marker, None), "{marker:?}");
        }
        assert_eq!(first.tab_label().as_deref(), Some("Failed 1"));
    }

    #[test]
    fn successful_or_statusless_completion_has_no_failure_badge() {
        let shell = SessionShellState::default();
        assert!(shell.set_marker("D;0", None));
        assert_eq!(shell.tab_label(), None);
        assert!(shell.set_marker("D", None));
        assert_eq!(shell.tab_label(), None);
    }

    #[test]
    fn prompt_navigation_is_bounded_ordered_and_private() {
        let shell = SessionShellState::default();
        for line in [25, 2, 12, 25] {
            shell.set_marker("A", Some(line));
        }

        assert_eq!(
            shell.prompt_offset(PromptDirection::Previous, 30, 0, 100),
            Some(5)
        );
        assert_eq!(
            shell.prompt_offset(PromptDirection::Previous, 30, 5, 100),
            Some(18)
        );
        assert_eq!(
            shell.prompt_offset(PromptDirection::Next, 30, 18, 100),
            Some(5)
        );
        assert_eq!(shell.prompt_offset(PromptDirection::Next, 30, 5, 100), None);

        shell.clear_prompt_marks();
        assert_eq!(
            shell.prompt_offset(PromptDirection::Previous, 30, 0, 100),
            None
        );
    }

    #[test]
    fn prompt_navigation_fails_closed_at_history_eviction() {
        let shell = SessionShellState::default();
        assert!(shell.set_marker("A", Some(2)));
        assert_eq!(
            shell.prompt_offset(PromptDirection::Previous, 10, 0, 10),
            None
        );
        assert_eq!(
            shell.prompt_offset(PromptDirection::Previous, 9, 0, 10),
            Some(7)
        );
    }
}
