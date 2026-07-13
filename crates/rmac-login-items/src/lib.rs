//! Platform-neutral login-item state and desktop-entry mutation rules.

use std::fmt;
use std::path::PathBuf;

pub const MAX_ITEMS: usize = 512;
pub const MAX_ISSUES: usize = 128;
pub const MAX_ENTRY_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub source: PathBuf,
    pub enabled: bool,
    pub applies_to_session: bool,
    pub session_detail: Option<String>,
    pub user_owned: bool,
    pub managed_override: bool,
    pub can_toggle: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub file: String,
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub items: Vec<Item>,
    pub issues: Vec<Issue>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedEntry {
    pub name: String,
    pub hidden: bool,
    pub only_show_in: Vec<String>,
    pub not_show_in: Vec<String>,
    pub try_exec: Option<String>,
    pub managed_override: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidEntry,
    Unavailable,
    Mutation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    detail: String,
}

impl Error {
    pub fn new(kind: ErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for Error {}

pub trait Service {
    fn snapshot(&self) -> Result<Snapshot, Error>;
    fn set_enabled(&self, id: &str, enabled: bool) -> Result<Snapshot, Error>;
}

pub fn validate_id(id: &str) -> Result<(), Error> {
    if id.is_empty()
        || id.len() > 255
        || !id.ends_with(".desktop")
        || id.contains('/')
        || id.contains('\\')
        || id == ".desktop"
        || id.chars().any(char::is_control)
    {
        Err(Error::new(
            ErrorKind::InvalidEntry,
            "the autostart entry identifier is invalid",
        ))
    } else {
        Ok(())
    }
}

pub fn parse_entry(contents: &str) -> Result<ParsedEntry, Error> {
    if contents.len() > MAX_ENTRY_BYTES {
        return Err(Error::new(ErrorKind::InvalidEntry, "entry is too large"));
    }
    let mut in_group = false;
    let mut found_group = false;
    let mut entry_type = None;
    let mut name = None;
    let mut hidden = false;
    let mut only = Vec::new();
    let mut not = Vec::new();
    let mut managed = false;
    let mut try_exec = None;
    for raw in contents.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_group = line == "[Desktop Entry]";
            found_group |= in_group;
            continue;
        }
        if !in_group || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "Type" => {
                entry_type.get_or_insert(value);
            }
            "Name" => {
                name.get_or_insert(value);
            }
            "Hidden" => hidden = value.eq_ignore_ascii_case("true"),
            "OnlyShowIn" => only = split_list(value),
            "NotShowIn" => not = split_list(value),
            "TryExec" => {
                try_exec.get_or_insert(value);
            }
            "X-rmac-ManagedHidden" => managed = value.eq_ignore_ascii_case("true"),
            _ => continue,
        };
    }
    if !found_group || entry_type != Some("Application") {
        return Err(Error::new(
            ErrorKind::InvalidEntry,
            "missing [Desktop Entry] Type=Application",
        ));
    }
    let name = name
        .filter(|name| !name.is_empty())
        .ok_or_else(|| Error::new(ErrorKind::InvalidEntry, "desktop entry has no display name"))?;
    if !only.is_empty() && !not.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidEntry,
            "OnlyShowIn and NotShowIn cannot both be present",
        ));
    }
    Ok(ParsedEntry {
        name: name.to_owned(),
        hidden,
        only_show_in: only,
        not_show_in: not,
        try_exec: try_exec.map(str::to_owned),
        managed_override: managed,
    })
}

pub fn applies_to_session(entry: &ParsedEntry, desktops: &[String]) -> bool {
    !desktops
        .iter()
        .any(|desktop| entry.not_show_in.iter().any(|name| name == desktop))
        && (entry.only_show_in.is_empty()
            || desktops
                .iter()
                .any(|desktop| entry.only_show_in.iter().any(|name| name == desktop)))
}

pub fn with_hidden(contents: &str, hidden: bool, managed: bool) -> Result<String, Error> {
    parse_entry(contents)?;
    let mut output = String::new();
    let mut in_group = false;
    let mut inserted = false;
    for raw in contents.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            if in_group && !inserted {
                append_hidden(&mut output, hidden, managed);
                inserted = true;
            }
            in_group = line == "[Desktop Entry]";
        }
        if in_group && (line.starts_with("Hidden=") || line.starts_with("X-rmac-ManagedHidden=")) {
            continue;
        }
        output.push_str(raw);
        output.push('\n');
    }
    if in_group && !inserted {
        append_hidden(&mut output, hidden, managed);
    }
    Ok(output)
}

fn append_hidden(output: &mut String, hidden: bool, managed: bool) {
    output.push_str(if hidden {
        "Hidden=true\n"
    } else {
        "Hidden=false\n"
    });
    if managed {
        output.push_str("X-rmac-ManagedHidden=true\n");
    }
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(';')
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &str =
        "[Desktop Entry]\nType=Application\nName=Demo\nOnlyShowIn=rmac;GNOME;\nExec=demo\n";

    #[test]
    fn parser_and_session_filter_follow_xdg_keys() {
        let entry = parse_entry(ENTRY).unwrap();
        assert!(applies_to_session(&entry, &["rmac".into()]));
        assert!(!applies_to_session(&entry, &["KDE".into()]));
        assert!(validate_id("org.example.Demo.desktop").is_ok());
        assert!(validate_id("../Demo.desktop").is_err());
    }

    #[test]
    fn hidden_update_preserves_entry_and_is_idempotent() {
        let hidden = with_hidden(ENTRY, true, true).unwrap();
        assert!(hidden.contains("Exec=demo"));
        assert!(hidden.contains("Hidden=true"));
        assert!(hidden.contains("X-rmac-ManagedHidden=true"));
        let enabled = with_hidden(&hidden, false, false).unwrap();
        assert_eq!(enabled.matches("Hidden=").count(), 1);
        assert!(!enabled.contains("X-rmac-ManagedHidden"));
    }

    #[test]
    fn malformed_entries_are_rejected_without_rewrite() {
        assert_eq!(
            parse_entry("[Desktop Entry]\nName=No type\n")
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidEntry
        );
    }
}
