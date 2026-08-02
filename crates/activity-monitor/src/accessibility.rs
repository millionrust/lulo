//! Bounded process-table, confirmation-dialog, and live-feedback semantics.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessColumn {
    Pid,
    Name,
    Cpu,
    Memory,
    Energy,
    Disk,
    ParentPid,
    User,
    VirtualMemory,
    RunTime,
    Status,
}

impl ProcessColumn {
    pub const ALL: [Self; 11] = [
        Self::Pid,
        Self::Name,
        Self::Cpu,
        Self::Memory,
        Self::Energy,
        Self::Disk,
        Self::ParentPid,
        Self::User,
        Self::VirtualMemory,
        Self::RunTime,
        Self::Status,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Pid => "PID",
            Self::Name => "Process Name",
            Self::Cpu => "% CPU",
            Self::Memory => "Memory",
            Self::Energy => "Energy",
            Self::Disk => "Disk I/O",
            Self::ParentPid => "Parent PID",
            Self::User => "User",
            Self::VirtualMemory => "Virtual Mem",
            Self::RunTime => "Run Time",
            Self::Status => "Status",
        }
    }
}

pub trait ProcessRowSemantics {
    fn pid(&self) -> u32;
    fn name(&self) -> &str;
    fn cell_text(&self, column: ProcessColumn) -> String;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleProcessColumn {
    pub name: &'static str,
    pub sort: Option<SortDirection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleProcessRow {
    pub pid: u32,
    pub name: String,
    pub label: String,
    pub cells: Vec<String>,
    pub selected: bool,
    pub actions: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessTableAccessibilitySnapshot {
    pub name: String,
    pub columns: Vec<AccessibleProcessColumn>,
    pub rows: Vec<AccessibleProcessRow>,
    pub selected_row: Option<usize>,
    pub row_count: usize,
    pub column_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessActionKind {
    Quit,
    ForceQuit,
}

impl ProcessActionKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Quit => "Quit",
            Self::ForceQuit => "Force Quit",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogActionKind {
    Cancel,
    Default,
    Destructive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleDialogAction {
    pub name: &'static str,
    pub kind: DialogActionKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessActionDialogSnapshot {
    pub title: String,
    pub description: String,
    pub actions: Vec<AccessibleDialogAction>,
    pub initial_focus: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LivePoliteness {
    Polite,
    Assertive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveAnnouncementSnapshot {
    pub text: String,
    pub politeness: LivePoliteness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityProjectionError {
    RowLimit,
    InvalidColumns,
    DuplicateProcess,
    InvalidText,
    TextLimit,
}

pub const MAX_ACCESSIBLE_PROCESS_ROWS: usize = 300;
pub const MAX_ACCESSIBLE_PROCESS_COLUMNS: usize = ProcessColumn::ALL.len();
pub const MAX_ACCESSIBLE_PROCESS_TEXT_BYTES: usize = 512 * 1024;

pub fn project_process_table<T: ProcessRowSemantics>(
    name: &str,
    rows: &[T],
    columns: &[ProcessColumn],
    selected_pid: Option<u32>,
    sort_column: ProcessColumn,
    sort_direction: SortDirection,
) -> Result<ProcessTableAccessibilitySnapshot, AccessibilityProjectionError> {
    if name.trim().is_empty() {
        return Err(AccessibilityProjectionError::InvalidText);
    }
    if rows.len() > MAX_ACCESSIBLE_PROCESS_ROWS {
        return Err(AccessibilityProjectionError::RowLimit);
    }
    if columns.is_empty()
        || columns.len() > MAX_ACCESSIBLE_PROCESS_COLUMNS
        || !columns.contains(&ProcessColumn::Name)
        || !columns.contains(&sort_column)
        || columns
            .iter()
            .enumerate()
            .any(|(index, column)| columns[..index].contains(column))
    {
        return Err(AccessibilityProjectionError::InvalidColumns);
    }

    let mut text_bytes = name.len();
    let projected_columns = columns
        .iter()
        .map(|column| {
            let name = column.title();
            text_bytes = text_bytes.saturating_add(name.len());
            AccessibleProcessColumn {
                name,
                sort: (*column == sort_column).then_some(sort_direction),
            }
        })
        .collect();
    let mut projected_rows = Vec::with_capacity(rows.len());
    let mut seen_pids = Vec::with_capacity(rows.len());
    let mut selected_row = None;
    for (index, row) in rows.iter().enumerate() {
        let pid = row.pid();
        if seen_pids.contains(&pid) {
            return Err(AccessibilityProjectionError::DuplicateProcess);
        }
        seen_pids.push(pid);
        if row.name().trim().is_empty() {
            return Err(AccessibilityProjectionError::InvalidText);
        }
        let process_name = row.name().to_string();
        let label = format!("{} (PID {pid})", row.name());
        let cells: Vec<String> = columns
            .iter()
            .map(|column| row.cell_text(*column))
            .collect();
        text_bytes = text_bytes
            .saturating_add(process_name.len())
            .saturating_add(label.len())
            .saturating_add(
                cells
                    .iter()
                    .fold(0usize, |total, cell| total.saturating_add(cell.len())),
            )
            .saturating_add("Inspect".len() + "Quit".len() + "Force Quit".len());
        let selected = selected_pid == Some(pid);
        if selected {
            selected_row = Some(index);
        }
        projected_rows.push(AccessibleProcessRow {
            pid,
            name: process_name,
            label,
            cells,
            selected,
            actions: vec!["Inspect", "Quit", "Force Quit"],
        });
    }
    if text_bytes > MAX_ACCESSIBLE_PROCESS_TEXT_BYTES {
        return Err(AccessibilityProjectionError::TextLimit);
    }

    Ok(ProcessTableAccessibilitySnapshot {
        name: name.to_string(),
        columns: projected_columns,
        rows: projected_rows,
        selected_row,
        row_count: rows.len(),
        column_count: columns.len(),
    })
}

pub fn project_process_action_dialog(
    process_name: &str,
    pid: u32,
    kind: ProcessActionKind,
) -> Result<ProcessActionDialogSnapshot, AccessibilityProjectionError> {
    if process_name.trim().is_empty() {
        return Err(AccessibilityProjectionError::InvalidText);
    }
    let label = kind.label();
    let title = format!("{label} Process");
    let description = format!(
        "Do you want to {} the process “{process_name}” (PID {pid})?",
        label.to_lowercase()
    );
    if title
        .len()
        .saturating_add(description.len())
        .saturating_add("Cancel".len())
        .saturating_add(label.len())
        > MAX_ACCESSIBLE_PROCESS_TEXT_BYTES
    {
        return Err(AccessibilityProjectionError::TextLimit);
    }
    Ok(ProcessActionDialogSnapshot {
        title,
        description,
        actions: vec![
            AccessibleDialogAction {
                name: "Cancel",
                kind: DialogActionKind::Cancel,
            },
            AccessibleDialogAction {
                name: label,
                kind: if kind == ProcessActionKind::ForceQuit {
                    DialogActionKind::Destructive
                } else {
                    DialogActionKind::Default
                },
            },
        ],
        initial_focus: 0,
    })
}

pub fn project_live_feedback(
    title: &str,
    detail: &str,
    success: bool,
) -> Result<LiveAnnouncementSnapshot, AccessibilityProjectionError> {
    if title.trim().is_empty() || detail.trim().is_empty() {
        return Err(AccessibilityProjectionError::InvalidText);
    }
    if title.len().saturating_add(detail.len()).saturating_add(2)
        > MAX_ACCESSIBLE_PROCESS_TEXT_BYTES
    {
        return Err(AccessibilityProjectionError::TextLimit);
    }
    Ok(LiveAnnouncementSnapshot {
        text: format!("{title}. {detail}"),
        politeness: if success {
            LivePoliteness::Polite
        } else {
            LivePoliteness::Assertive
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Row {
        pid: u32,
        name: &'static str,
        cpu: &'static str,
    }

    impl ProcessRowSemantics for Row {
        fn pid(&self) -> u32 {
            self.pid
        }

        fn name(&self) -> &str {
            self.name
        }

        fn cell_text(&self, column: ProcessColumn) -> String {
            match column {
                ProcessColumn::Pid => self.pid.to_string(),
                ProcessColumn::Name => self.name.to_string(),
                ProcessColumn::Cpu => self.cpu.to_string(),
                _ => String::new(),
            }
        }
    }

    #[test]
    fn process_table_preserves_visible_order_sort_selection_and_actions() {
        let rows = [
            Row {
                pid: 7,
                name: "éditeur",
                cpu: "3.5",
            },
            Row {
                pid: 9,
                name: "worker",
                cpu: "1.0",
            },
        ];
        let snapshot = project_process_table(
            "Processes",
            &rows,
            &[ProcessColumn::Name, ProcessColumn::Pid, ProcessColumn::Cpu],
            Some(7),
            ProcessColumn::Cpu,
            SortDirection::Descending,
        )
        .unwrap();

        assert_eq!(snapshot.row_count, 2);
        assert_eq!(snapshot.column_count, 3);
        assert_eq!(snapshot.selected_row, Some(0));
        assert_eq!(snapshot.columns[2].sort, Some(SortDirection::Descending));
        assert_eq!(snapshot.rows[0].cells, ["éditeur", "7", "3.5"]);
        assert_eq!(snapshot.rows[0].label, "éditeur (PID 7)");
        assert_eq!(snapshot.rows[0].actions, ["Inspect", "Quit", "Force Quit"]);
    }

    #[test]
    fn table_refuses_ambiguous_columns_processes_and_unbounded_rows() {
        let row = Row {
            pid: 7,
            name: "worker",
            cpu: "1.0",
        };
        assert_eq!(
            project_process_table(
                "Processes",
                std::slice::from_ref(&row),
                &[ProcessColumn::Name, ProcessColumn::Name],
                None,
                ProcessColumn::Name,
                SortDirection::Ascending,
            ),
            Err(AccessibilityProjectionError::InvalidColumns)
        );
        assert_eq!(
            project_process_table(
                "Processes",
                &[&row, &row],
                &[ProcessColumn::Name],
                None,
                ProcessColumn::Name,
                SortDirection::Ascending,
            ),
            Err(AccessibilityProjectionError::DuplicateProcess)
        );
        let many: Vec<_> = (0..=MAX_ACCESSIBLE_PROCESS_ROWS)
            .map(|pid| Row {
                pid: pid as u32,
                name: "worker",
                cpu: "1.0",
            })
            .collect();
        assert_eq!(
            project_process_table(
                "Processes",
                &many,
                &[ProcessColumn::Name],
                None,
                ProcessColumn::Name,
                SortDirection::Ascending,
            ),
            Err(AccessibilityProjectionError::RowLimit)
        );
    }

    impl<T: ProcessRowSemantics + ?Sized> ProcessRowSemantics for &T {
        fn pid(&self) -> u32 {
            (**self).pid()
        }

        fn name(&self) -> &str {
            (**self).name()
        }

        fn cell_text(&self, column: ProcessColumn) -> String {
            (**self).cell_text(column)
        }
    }

    #[test]
    fn action_dialog_and_feedback_define_focus_danger_and_live_priority() {
        let quit = project_process_action_dialog("worker", 42, ProcessActionKind::Quit).unwrap();
        assert_eq!(quit.initial_focus, 0);
        assert_eq!(quit.actions[0].kind, DialogActionKind::Cancel);
        assert_eq!(quit.actions[1].kind, DialogActionKind::Default);

        let force =
            project_process_action_dialog("worker", 42, ProcessActionKind::ForceQuit).unwrap();
        assert_eq!(force.actions[1].kind, DialogActionKind::Destructive);
        assert!(force.description.contains("PID 42"));

        let success = project_live_feedback(
            "Quit request sent",
            "The signal was delivered; process exit is not confirmed.",
            true,
        )
        .unwrap();
        assert_eq!(success.politeness, LivePoliteness::Polite);
        let failure = project_live_feedback(
            "Quit was not sent",
            "The operating system rejected the request.",
            false,
        )
        .unwrap();
        assert_eq!(failure.politeness, LivePoliteness::Assertive);
        assert_eq!(
            project_live_feedback("", "No request was sent.", false),
            Err(AccessibilityProjectionError::InvalidText)
        );
    }
}
