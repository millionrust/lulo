//! Bounded FinalTerm/OSC 133 command-phase and prompt-mark state for one session.

use std::sync::{Arc, Mutex};

const MAX_MARKER_BYTES: usize = 32;
const MAX_PROMPT_MARKS: usize = 512;
const MAX_COMMAND_RANGES: usize = 256;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CommandRangeKind {
    Command,
    Output,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct MarkerPosition {
    pub(super) line: usize,
    pub(super) column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MarkerRange {
    pub(super) start: MarkerPosition,
    pub(super) end: MarkerPosition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingCommand {
    prompt_start: MarkerPosition,
    command_start: Option<MarkerPosition>,
    output_start: Option<MarkerPosition>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CommandRange {
    prompt_start: MarkerPosition,
    command_start: MarkerPosition,
    output_start: MarkerPosition,
    output_end: MarkerPosition,
}

#[derive(Debug, Default)]
struct ShellState {
    phase: CommandPhase,
    /// Retained-grid line coordinates only; no prompt or command text is stored.
    prompt_lines: Vec<usize>,
    pending_command: Option<PendingCommand>,
    command_ranges: Vec<CommandRange>,
}

#[derive(Clone, Default)]
pub(super) struct SessionShellState {
    state: Arc<Mutex<ShellState>>,
}

impl SessionShellState {
    /// Accept one complete OSC 133 payload after the command separator. A
    /// prompt coordinate is supplied only when the parser is on the primary
    /// grid and its retained history has not reached the eviction boundary.
    pub(super) fn set_marker(&self, marker: &str, position: Option<MarkerPosition>) -> bool {
        let Some(next) = parse_marker(marker) else {
            return false;
        };
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let phase_changed = state.phase != next;
        state.phase = next;

        let coordinates_changed = match next {
            CommandPhase::Prompt => {
                state.pending_command = position.map(|prompt_start| PendingCommand {
                    prompt_start,
                    command_start: None,
                    output_start: None,
                });
                let Some(line) = position
                    .map(|position| position.line)
                    .filter(|line| !state.prompt_lines.contains(line))
                else {
                    return phase_changed;
                };
                if state.prompt_lines.len() == MAX_PROMPT_MARKS {
                    state.prompt_lines.remove(0);
                }
                state.prompt_lines.push(line);
                true
            }
            CommandPhase::Input => {
                let Some(mut pending) = state.pending_command.take() else {
                    return phase_changed;
                };
                let Some(position) = position.filter(|position| {
                    pending.command_start.is_none() && *position >= pending.prompt_start
                }) else {
                    return phase_changed;
                };
                pending.command_start = Some(position);
                state.pending_command = Some(pending);
                true
            }
            CommandPhase::Running => {
                let Some(mut pending) = state.pending_command.take() else {
                    return phase_changed;
                };
                let Some(position) = position.filter(|position| {
                    pending.command_start.is_some_and(|command_start| {
                        pending.output_start.is_none() && *position >= command_start
                    })
                }) else {
                    return phase_changed;
                };
                pending.output_start = Some(position);
                state.pending_command = Some(pending);
                true
            }
            CommandPhase::Finished(_) => {
                let Some(pending) = state.pending_command.take() else {
                    return phase_changed;
                };
                let Some((command_start, output_start, output_end)) = pending
                    .command_start
                    .zip(pending.output_start)
                    .zip(position)
                    .map(|((command_start, output_start), output_end)| {
                        (command_start, output_start, output_end)
                    })
                    .filter(|(command_start, output_start, output_end)| {
                        command_start <= output_start && output_start <= output_end
                    })
                else {
                    return phase_changed;
                };
                if state.command_ranges.len() == MAX_COMMAND_RANGES {
                    state.command_ranges.remove(0);
                }
                state.command_ranges.push(CommandRange {
                    prompt_start: pending.prompt_start,
                    command_start,
                    output_start,
                    output_end,
                });
                true
            }
            CommandPhase::Unknown => false,
        };
        phase_changed || coordinates_changed
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

    pub(super) fn clear_grid_marks(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.prompt_lines.clear();
            state.pending_command = None;
            state.command_ranges.clear();
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

    pub(super) fn command_range(
        &self,
        kind: CommandRangeKind,
        history_size: usize,
        display_offset: usize,
        history_limit: usize,
        columns: usize,
    ) -> Option<MarkerRange> {
        if columns == 0 || history_limit == 0 || history_size >= history_limit {
            return None;
        }
        let viewport_top = history_size.saturating_sub(display_offset);
        let state = self.state.lock().ok()?;
        let range = if display_offset == 0 {
            state.command_ranges.last()
        } else {
            state
                .command_ranges
                .iter()
                .rev()
                .find(|range| range.prompt_start.line <= viewport_top)
        }?;
        let (start, end_exclusive) = match kind {
            CommandRangeKind::Command => (range.command_start, range.output_start),
            CommandRangeKind::Output => (range.output_start, range.output_end),
        };
        inclusive_range(start, end_exclusive, columns)
    }
}

fn inclusive_range(
    start: MarkerPosition,
    end_exclusive: MarkerPosition,
    columns: usize,
) -> Option<MarkerRange> {
    if columns == 0 || start >= end_exclusive {
        return None;
    }
    let end = if end_exclusive.column > 0 {
        MarkerPosition {
            line: end_exclusive.line,
            column: end_exclusive.column.saturating_sub(1).min(columns - 1),
        }
    } else {
        MarkerPosition {
            line: end_exclusive.line.checked_sub(1)?,
            column: columns - 1,
        }
    };
    (start <= end).then_some(MarkerRange { start, end })
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

    fn position(line: usize, column: usize) -> MarkerPosition {
        MarkerPosition { line, column }
    }

    #[test]
    fn reports_running_and_failure_without_retaining_command_text() {
        let shell = SessionShellState::default();
        assert!(shell.set_marker("C", None));
        assert_eq!(shell.tab_label().as_deref(), Some("Running"));
        assert!(shell.set_marker("D;127", None));
        assert_eq!(shell.tab_label().as_deref(), Some("Failed 127"));
        assert!(shell.set_marker("A", Some(position(4, 0))));
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
            shell.set_marker("A", Some(position(line, 0)));
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

        shell.clear_grid_marks();
        assert_eq!(
            shell.prompt_offset(PromptDirection::Previous, 30, 0, 100),
            None
        );
    }

    #[test]
    fn prompt_navigation_fails_closed_at_history_eviction() {
        let shell = SessionShellState::default();
        assert!(shell.set_marker("A", Some(position(2, 0))));
        assert_eq!(
            shell.prompt_offset(PromptDirection::Previous, 10, 0, 10),
            None
        );
        assert_eq!(
            shell.prompt_offset(PromptDirection::Previous, 9, 0, 10),
            Some(7)
        );
    }

    #[test]
    fn complete_marker_sequences_produce_private_command_and_output_ranges() {
        let shell = SessionShellState::default();
        assert!(shell.set_marker("A", Some(position(2, 0))));
        assert!(shell.set_marker("B", Some(position(2, 4))));
        assert!(shell.set_marker("C", Some(position(3, 0))));
        assert!(shell.set_marker("D;0", Some(position(5, 0))));

        assert_eq!(
            shell.command_range(CommandRangeKind::Command, 10, 0, 100, 10),
            Some(MarkerRange {
                start: position(2, 4),
                end: position(2, 9),
            })
        );
        assert_eq!(
            shell.command_range(CommandRangeKind::Output, 10, 0, 100, 10),
            Some(MarkerRange {
                start: position(3, 0),
                end: position(4, 9),
            })
        );

        shell.set_marker("A", Some(position(7, 0)));
        shell.set_marker("B", Some(position(7, 2)));
        shell.set_marker("C", Some(position(8, 0)));
        shell.set_marker("D", Some(position(9, 3)));
        assert_eq!(
            shell.command_range(CommandRangeKind::Output, 10, 0, 100, 10),
            Some(MarkerRange {
                start: position(8, 0),
                end: position(9, 2),
            })
        );
        assert_eq!(
            shell.command_range(CommandRangeKind::Output, 10, 8, 100, 10),
            Some(MarkerRange {
                start: position(3, 0),
                end: position(4, 9),
            })
        );
    }

    #[test]
    fn ranges_require_ordered_complete_markers_and_clear_with_grid_identity() {
        let shell = SessionShellState::default();
        shell.set_marker("A", Some(position(4, 0)));
        shell.set_marker("C", Some(position(5, 0)));
        shell.set_marker("D", Some(position(6, 0)));
        assert_eq!(
            shell.command_range(CommandRangeKind::Output, 10, 0, 100, 10),
            None
        );

        shell.set_marker("A", Some(position(5, 0)));
        shell.set_marker("B", Some(position(5, 2)));
        shell.set_marker("B", Some(position(5, 3)));
        shell.set_marker("C", Some(position(6, 0)));
        shell.set_marker("D", Some(position(6, 4)));
        assert_eq!(
            shell.command_range(CommandRangeKind::Command, 10, 0, 100, 10),
            None
        );

        shell.set_marker("A", Some(position(7, 0)));
        shell.set_marker("B", Some(position(7, 2)));
        shell.set_marker("C", Some(position(8, 0)));
        shell.set_marker("D", Some(position(9, 3)));
        assert!(shell
            .command_range(CommandRangeKind::Output, 10, 0, 100, 10)
            .is_some());
        shell.clear_grid_marks();
        assert_eq!(
            shell.command_range(CommandRangeKind::Output, 10, 0, 100, 10),
            None
        );
    }
}
