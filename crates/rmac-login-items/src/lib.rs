//! Platform-neutral login-item state and desktop-entry mutation rules.

use std::fmt;
use std::path::PathBuf;

pub const MAX_ITEMS: usize = 512;
pub const MAX_BACKGROUND_SERVICES: usize = 512;
pub const MAX_ISSUES: usize = 128;
pub const MAX_ENTRY_BYTES: usize = 256 * 1024;
const MAX_ERROR_BYTES: usize = 512;
const MAX_DISPLAY_FIELD_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable,
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddPreview {
    pub source: PathBuf,
    pub id: String,
    pub name: String,
    pub command: String,
    pub replacing: bool,
    source_contents: Vec<u8>,
    target_contents: Option<Vec<u8>>,
}

impl AddPreview {
    pub fn new(
        source: PathBuf,
        id: String,
        name: String,
        command: String,
        source_contents: Vec<u8>,
        target_contents: Option<Vec<u8>>,
    ) -> Self {
        Self {
            source,
            id,
            name,
            command,
            replacing: target_contents.is_some(),
            source_contents,
            target_contents,
        }
    }

    pub fn source_contents(&self) -> &[u8] {
        &self.source_contents
    }

    pub fn target_contents(&self) -> Option<&[u8]> {
        self.target_contents.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovePreview {
    pub id: String,
    pub name: String,
    pub source: PathBuf,
    contents: Vec<u8>,
}

impl RemovePreview {
    pub fn new(id: String, name: String, source: PathBuf, contents: Vec<u8>) -> Self {
        Self {
            id,
            name,
            source,
            contents,
        }
    }

    pub fn contents(&self) -> &[u8] {
        &self.contents
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitFileState {
    Enabled,
    Disabled,
    Linked,
    Runtime,
    Masked,
    Static,
    Other,
}

impl UnitFileState {
    pub fn from_systemd(value: &str) -> Self {
        match value {
            "enabled" => Self::Enabled,
            "disabled" => Self::Disabled,
            "linked" => Self::Linked,
            "enabled-runtime" | "linked-runtime" => Self::Runtime,
            "masked" | "masked-runtime" => Self::Masked,
            "static" | "indirect" | "generated" | "transient" => Self::Static,
            _ => Self::Other,
        }
    }

    pub fn enabled(self) -> bool {
        matches!(self, Self::Enabled | Self::Linked | Self::Runtime)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Enabled => "Enabled",
            Self::Disabled => "Disabled",
            Self::Linked => "Linked",
            Self::Runtime => "Runtime only",
            Self::Masked => "Masked",
            Self::Static => "Static",
            Self::Other => "Unmanaged state",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundService {
    pub id: String,
    pub name: String,
    pub state: UnitFileState,
    pub enabled: bool,
    pub can_toggle: bool,
    pub detail: String,
    pub user_owned: bool,
    pub source: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub items: Vec<Item>,
    pub background_services: Vec<BackgroundService>,
    pub background_services_truncated: bool,
    pub background_services_error: Option<String>,
    pub issues: Vec<Issue>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedEntry {
    pub name: String,
    pub command: String,
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
    Conflict,
    Mutation,
    Mismatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    detail: String,
}

impl Error {
    pub fn new(kind: ErrorKind, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self {
            kind,
            detail: bounded_text(&detail),
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
    fn set_background_enabled(&self, id: &str, enabled: bool) -> Result<Snapshot, Error>;
    fn prepare_add(&self, source: &std::path::Path) -> Result<AddPreview, Error>;
    fn add(&self, preview: &AddPreview) -> Result<Snapshot, Error>;
    fn prepare_remove(&self, id: &str) -> Result<RemovePreview, Error>;
    fn remove(&self, preview: &RemovePreview) -> Result<Snapshot, Error>;
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

pub fn validate_service_id(id: &str) -> Result<(), Error> {
    if id.is_empty()
        || id.len() > 255
        || !id.ends_with(".service")
        || id.contains('/')
        || id.contains('\\')
        || id == ".service"
        || id.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | '@' | ':'))
        })
    {
        Err(Error::new(
            ErrorKind::InvalidEntry,
            "the systemd user service identifier is invalid",
        ))
    } else {
        Ok(())
    }
}

pub fn background_service(
    id: &str,
    raw_state: &str,
    user_owned: bool,
    source: Option<PathBuf>,
) -> Result<Option<BackgroundService>, Error> {
    validate_service_id(id)?;
    let state = UnitFileState::from_systemd(raw_state);
    let visible = user_owned
        || matches!(
            state,
            UnitFileState::Enabled
                | UnitFileState::Linked
                | UnitFileState::Runtime
                | UnitFileState::Masked
        );
    if !visible {
        return Ok(None);
    }
    let protected = id.starts_with("rmac-");
    let can_toggle = user_owned
        && !protected
        && matches!(
            state,
            UnitFileState::Enabled | UnitFileState::Disabled | UnitFileState::Linked
        );
    let name = id
        .strip_suffix(".service")
        .unwrap_or(id)
        .replace(['-', '_'], " ");
    let detail = if protected {
        "Required by the rmac session".into()
    } else if state == UnitFileState::Runtime {
        "Runtime-only state is read-only; it ends at logout or reboot".into()
    } else if state == UnitFileState::Masked {
        "Masked services must be reviewed and unmasked outside this pane".into()
    } else if state == UnitFileState::Static || state == UnitFileState::Other {
        "This unit has no safe persistent enable/disable transition".into()
    } else if user_owned {
        "User-installed systemd service".into()
    } else {
        "System-provided user service".into()
    };
    Ok(Some(BackgroundService {
        id: id.into(),
        name,
        state,
        enabled: state.enabled(),
        can_toggle,
        detail,
        user_owned,
        source,
    }))
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
    let mut exec = None;
    let mut dbus_activatable = false;
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
            "Exec" => {
                exec.get_or_insert(value);
            }
            "DBusActivatable" => dbus_activatable = value.eq_ignore_ascii_case("true"),
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
    validate_display_field(name, "desktop entry name")?;
    let command = exec
        .map(str::to_owned)
        .or_else(|| dbus_activatable.then(|| "D-Bus application activation".into()))
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidEntry,
                "desktop entry has neither Exec nor D-Bus activation",
            )
        })?;
    validate_display_field(&command, "desktop entry command")?;
    if let Some(value) = try_exec {
        validate_display_field(value, "desktop entry TryExec value")?;
    }
    if !only.is_empty() && !not.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidEntry,
            "OnlyShowIn and NotShowIn cannot both be present",
        ));
    }
    Ok(ParsedEntry {
        name: name.to_owned(),
        command,
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

fn validate_display_field(value: &str, label: &str) -> Result<(), Error> {
    if value.is_empty()
        || value.len() > MAX_DISPLAY_FIELD_BYTES
        || value.chars().any(char::is_control)
    {
        Err(Error::new(
            ErrorKind::InvalidEntry,
            format!("{label} is invalid or too large"),
        ))
    } else {
        Ok(())
    }
}

fn bounded_text(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut end = normalized.len().min(MAX_ERROR_BYTES);
    while !normalized.is_char_boundary(end) {
        end -= 1;
    }
    normalized[..end].trim().to_string()
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
        assert!(
            parse_entry("[Desktop Entry]\nType=Application\nName=Demo\nExec=bad\tcommand\n")
                .is_err()
        );
        assert!(parse_entry(&format!(
            "[Desktop Entry]\nType=Application\nName=Demo\nExec={}\n",
            "x".repeat(MAX_DISPLAY_FIELD_BYTES + 1)
        ))
        .is_err());
    }

    #[test]
    fn errors_are_bounded_and_control_normalized() {
        let error = Error::new(ErrorKind::Mutation, format!("{}\nsecret", "x".repeat(600)));
        assert!(error.to_string().len() <= MAX_ERROR_BYTES);
        assert!(!error.to_string().contains('\n'));
    }

    #[test]
    fn systemd_states_expose_only_safe_persistent_transitions() {
        let enabled = background_service("example.service", "enabled", true, None)
            .unwrap()
            .unwrap();
        assert!(enabled.enabled);
        assert!(enabled.can_toggle);
        assert!(
            background_service("unused.service", "disabled", false, None)
                .unwrap()
                .is_none()
        );
        assert!(
            background_service("custom.service", "disabled", true, None)
                .unwrap()
                .unwrap()
                .can_toggle
        );
        assert!(
            !background_service("rmac-dock.service", "enabled", true, None)
                .unwrap()
                .unwrap()
                .can_toggle
        );
        assert!(
            !background_service("temporary.service", "enabled-runtime", true, None)
                .unwrap()
                .unwrap()
                .can_toggle
        );
        assert!(validate_service_id("../bad.service").is_err());
    }

    #[test]
    fn system_services_are_read_only_without_user_owned_unit_authority() {
        let system = background_service("system-agent.service", "enabled", false, None)
            .unwrap()
            .unwrap();
        assert!(!system.can_toggle);
        let user = background_service("user-agent.service", "enabled", true, None)
            .unwrap()
            .unwrap();
        assert!(user.can_toggle);
    }
}
