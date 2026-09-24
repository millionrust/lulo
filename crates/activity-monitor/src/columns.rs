use std::path::{Path, PathBuf};

use gpui::px;
use rmac_activity_monitor::accessibility::ProcessColumn;
use rmac_ui::Column;

use crate::metrics::Tab;
use crate::storage;

/// A column in the process table. The set is fixed and canonically ordered;
/// the column chooser toggles which ones are visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColKey {
    Pid,
    Name,
    Cpu,
    Mem,
    Energy,
    Disk,
    Ppid,
    User,
    Vmem,
    RunTime,
    Status,
    Threads,
}

impl ColKey {
    /// Canonical order, also the order shown in the chooser. Process Name
    /// leads, matching the Mac (MON-02); PID moves later rather than first.
    pub(crate) const ALL: [Self; 12] = [
        Self::Name,
        Self::Cpu,
        Self::Threads,
        Self::Mem,
        Self::Energy,
        Self::Disk,
        Self::User,
        Self::Pid,
        Self::Ppid,
        Self::Vmem,
        Self::RunTime,
        Self::Status,
    ];

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Pid => "pid",
            Self::Name => "name",
            Self::Cpu => "cpu",
            Self::Mem => "mem",
            Self::Energy => "energy",
            Self::Disk => "disk",
            Self::Ppid => "ppid",
            Self::User => "user",
            Self::Vmem => "vmem",
            Self::RunTime => "runtime",
            Self::Status => "status",
            Self::Threads => "threads",
        }
    }

    fn from_id(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|key| key.id() == value)
    }

    pub(crate) fn title(self) -> &'static str {
        ProcessColumn::from(self).title()
    }

    fn width(self) -> f32 {
        match self {
            Self::Pid => 72.0,
            Self::Name => 280.0,
            Self::Cpu => 96.0,
            Self::Mem => 110.0,
            // Wider than the other 96px columns: the header now reads
            // "Energy (Est.)" rather than plain "Energy".
            Self::Energy => 116.0,
            Self::Disk => 120.0,
            Self::Ppid => 96.0,
            Self::User => 130.0,
            Self::Vmem => 120.0,
            Self::RunTime => 110.0,
            Self::Status => 110.0,
            Self::Threads => 90.0,
        }
    }

    fn right_aligned(self) -> bool {
        matches!(
            self,
            Self::Pid
                | Self::Cpu
                | Self::Mem
                | Self::Energy
                | Self::Disk
                | Self::Ppid
                | Self::Vmem
                | Self::RunTime
                | Self::Threads
        )
    }

    /// The Process Name column is the human anchor and cannot be hidden.
    pub(crate) fn required(self) -> bool {
        matches!(self, Self::Name)
    }

    pub(crate) fn to_column(self) -> Column {
        let column = Column::new(self.id(), self.title())
            .width(px(self.width()))
            .sortable();
        if self.right_aligned() {
            column.text_right()
        } else {
            column
        }
    }
}

impl From<ColKey> for ProcessColumn {
    fn from(value: ColKey) -> Self {
        match value {
            ColKey::Pid => Self::Pid,
            ColKey::Name => Self::Name,
            ColKey::Cpu => Self::Cpu,
            ColKey::Mem => Self::Memory,
            ColKey::Energy => Self::Energy,
            ColKey::Disk => Self::Disk,
            ColKey::Ppid => Self::ParentPid,
            ColKey::User => Self::User,
            ColKey::Vmem => Self::VirtualMemory,
            ColKey::RunTime => Self::RunTime,
            ColKey::Status => Self::Status,
            ColKey::Threads => Self::Threads,
        }
    }
}

impl From<ProcessColumn> for ColKey {
    fn from(value: ProcessColumn) -> Self {
        match value {
            ProcessColumn::Pid => Self::Pid,
            ProcessColumn::Name => Self::Name,
            ProcessColumn::Cpu => Self::Cpu,
            ProcessColumn::Memory => Self::Mem,
            ProcessColumn::Energy => Self::Energy,
            ProcessColumn::Disk => Self::Disk,
            ProcessColumn::ParentPid => Self::Ppid,
            ProcessColumn::User => Self::User,
            ProcessColumn::VirtualMemory => Self::Vmem,
            ProcessColumn::RunTime => Self::RunTime,
            ProcessColumn::Status => Self::Status,
            ProcessColumn::Threads => Self::Threads,
        }
    }
}

/// Current and retired paths to the persisted visible-columns file.
fn config_paths() -> Result<(PathBuf, PathBuf), storage::Failure> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        storage::Failure::message(
            storage::Operation::ResolveConfigPath,
            Path::new("columns.txt"),
            "HOME is not set",
        )
    })?;
    #[cfg(target_os = "macos")]
    let root = home.join("Library/Application Support");
    #[cfg(not(target_os = "macos"))]
    let root = match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(path) if path.is_absolute() => path,
        _ => home.join(".config"),
    };
    Ok((
        root.join("rmac-system-monitor/columns.txt"),
        root.join("rmac-activity-monitor/columns.txt"),
    ))
}

/// The column set the app opens with, before any tab has been chosen — the
/// same as the CPU tab's default, since `MonitorView` starts on CPU.
pub(crate) fn default_visible() -> Vec<ColKey> {
    default_visible_for(Tab::Cpu)
}

/// Each tab's own default column set (MON-02): the Mac shows CPU-relevant
/// columns on the CPU tab, memory-relevant ones on the Memory tab, and so
/// on, rather than one shared list. Only genuinely available metrics are
/// offered — there is no per-process GPU, idle-wakeup or port count on
/// Linux, so those Mac-only columns are left out rather than faked.
pub(crate) fn default_visible_for(tab: Tab) -> Vec<ColKey> {
    let wanted: &[ColKey] = match tab {
        Tab::Cpu => &[
            ColKey::Name,
            ColKey::Cpu,
            ColKey::Threads,
            ColKey::RunTime,
            ColKey::Pid,
            ColKey::User,
        ],
        Tab::Memory => &[
            ColKey::Name,
            ColKey::Mem,
            ColKey::Vmem,
            ColKey::Threads,
            ColKey::Pid,
            ColKey::User,
        ],
        Tab::Energy => &[
            ColKey::Name,
            ColKey::Energy,
            ColKey::Cpu,
            ColKey::Pid,
            ColKey::User,
        ],
        Tab::Disk => &[ColKey::Name, ColKey::Disk, ColKey::Pid, ColKey::User],
        // Network has no process table (`Tab::has_process_table`); this
        // list is never shown but kept so every tab has one.
        Tab::Network => &[ColKey::Name, ColKey::Pid, ColKey::User],
    };
    ColKey::ALL
        .into_iter()
        .filter(|key| wanted.contains(key))
        .collect()
}

fn parse(content: &str) -> Result<Vec<ColKey>, String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("column preferences are empty".into());
    }
    let mut selected = Vec::new();
    for token in trimmed.split(',') {
        let id = token.trim();
        if id.is_empty() {
            return Err("column preferences contain an empty id".into());
        }
        let key = ColKey::from_id(id).ok_or_else(|| format!("unknown column id '{id}'"))?;
        if selected.contains(&key) {
            return Err(format!("column id '{id}' is duplicated"));
        }
        selected.push(key);
    }

    // Always restore the required name anchor and normalize display order to
    // the canonical table order, even if an older file used another order.
    Ok(ColKey::ALL
        .into_iter()
        .filter(|key| key.required() || selected.contains(key))
        .collect())
}

/// Load the visible column set, treating a missing file as first launch.
pub(crate) fn load() -> Result<Vec<ColKey>, storage::Failure> {
    let (path, legacy_path) = config_paths()?;
    for candidate in [&path, &legacy_path] {
        if let Some(content) = storage::load_optional(&storage::RealStorage, candidate)? {
            return parse(&content).map_err(|detail| {
                storage::Failure::message(storage::Operation::LoadColumns, candidate, detail)
            });
        }
    }
    Ok(default_visible())
}

fn format(columns: &[ColKey]) -> String {
    columns
        .iter()
        .map(|key| key.id())
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn save(columns: &[ColKey]) -> Result<(), storage::Failure> {
    let (path, _legacy_path) = config_paths()?;
    storage::save(&storage::RealStorage, &path, format(columns))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_columns_round_trip_in_canonical_order() {
        // Process Name leads the canonical order (MON-02), not PID.
        let expected = vec![ColKey::Name, ColKey::Mem, ColKey::Pid, ColKey::Status];
        assert_eq!(parse(&format(&expected)).unwrap(), expected);
    }

    #[test]
    fn malformed_column_preferences_are_reported() {
        assert!(parse("").is_err());
        assert!(parse("name,unknown").is_err());
        assert!(parse("name,name").is_err());
        assert!(parse("name,").is_err());
    }

    #[test]
    fn required_name_column_is_restored_and_order_is_normalized() {
        assert_eq!(
            parse("status,pid").unwrap(),
            vec![ColKey::Name, ColKey::Pid, ColKey::Status]
        );
    }

    #[test]
    fn each_tab_gets_its_own_default_columns() {
        let cpu = default_visible_for(Tab::Cpu);
        let memory = default_visible_for(Tab::Memory);
        let energy = default_visible_for(Tab::Energy);
        let disk = default_visible_for(Tab::Disk);
        assert!(cpu.contains(&ColKey::Cpu) && cpu.contains(&ColKey::Threads));
        assert!(!cpu.contains(&ColKey::Energy));
        assert!(memory.contains(&ColKey::Mem) && !memory.contains(&ColKey::Cpu));
        assert!(energy.contains(&ColKey::Energy) && !energy.contains(&ColKey::Threads));
        assert!(disk.contains(&ColKey::Disk) && !disk.contains(&ColKey::Mem));
        // Every tab keeps the required Name anchor.
        for columns in [&cpu, &memory, &energy, &disk] {
            assert!(columns.contains(&ColKey::Name));
        }
        assert_eq!(default_visible(), cpu);
    }
}
