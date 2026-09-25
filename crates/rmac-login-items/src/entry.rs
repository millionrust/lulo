use std::path::PathBuf;

use crate::model::{MAX_DISPLAY_FIELD_BYTES, MAX_ERROR_BYTES};
use crate::{BackgroundService, Error, ErrorKind, ParsedEntry, UnitFileState, MAX_ENTRY_BYTES};

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
        "Required by the Lulo OS session".into()
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
    let mut no_display = false;
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
            "NoDisplay" => no_display = value.eq_ignore_ascii_case("true"),
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
        no_display,
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

pub(crate) fn bounded_text(value: &str) -> String {
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
