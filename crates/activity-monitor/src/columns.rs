use std::path::{Path, PathBuf};

use gpui::px;
use rmac_activity_monitor::accessibility::ProcessColumn;
use rmac_ui::Column;

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
}

impl ColKey {
    /// Canonical order, also the order shown in the chooser.
    pub(crate) const ALL: [Self; 11] = [
        Self::Pid,
        Self::Name,
        Self::Cpu,
        Self::Mem,
        Self::Energy,
        Self::Disk,
        Self::Ppid,
        Self::User,
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
        )
    }

    fn default_visible(self) -> bool {
        matches!(
            self,
            Self::Pid | Self::Name | Self::Cpu | Self::Mem | Self::Energy | Self::Disk
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

pub(crate) fn default_visible() -> Vec<ColKey> {
    ColKey::ALL
        .into_iter()
        .filter(|key| key.default_visible())
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
        let expected = vec![ColKey::Pid, ColKey::Name, ColKey::Mem, ColKey::Status];
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
            vec![ColKey::Pid, ColKey::Name, ColKey::Status]
        );
    }
}
