//! The privileged helper's command line and small system-file parsers,
//! kept pure so they are unit-tested on every platform.

use crate::{CapsLockAction, MacKeyboard, PhysicalLayout, Profile};

/// The helper's `apply` arguments for `target`.
pub fn helper_arguments(target: &MacKeyboard) -> Vec<String> {
    let flag = |value: bool| if value { "on" } else { "off" }.to_owned();
    vec![
        "apply".into(),
        "--shortcuts".into(),
        flag(target.shortcuts_in_all_apps),
        "--swap".into(),
        flag(target.layout.swap_command_option),
        "--caps".into(),
        target.layout.caps_lock.id().into(),
        "--option-characters".into(),
        flag(target.layout.option_characters),
    ]
}

/// Parse [`helper_arguments`] back. Anything else is rejected, because the
/// helper runs as root.
pub fn parse_helper_arguments(arguments: &[String]) -> Option<MacKeyboard> {
    let flag = |value: &str| match value {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    };
    match arguments {
        [shortcuts_key, shortcuts, swap_key, swap, caps_key, caps, option_key, option]
            if shortcuts_key == "--shortcuts"
                && swap_key == "--swap"
                && caps_key == "--caps"
                && option_key == "--option-characters" =>
        {
            Some(MacKeyboard {
                shortcuts_in_all_apps: flag(shortcuts)?,
                layout: PhysicalLayout {
                    swap_command_option: flag(swap)?,
                    caps_lock: CapsLockAction::from_id(caps)?,
                    option_characters: flag(option)?,
                },
            })
        }
        _ => None,
    }
}

/// The GID of `group` in an `/etc/group` file.
pub fn parse_group_id(source: &str, group: &str) -> Option<u32> {
    source.lines().find_map(|line| {
        let mut fields = line.split(':');
        (fields.next()? == group)
            .then(|| fields.nth(1)?.parse().ok())
            .flatten()
    })
}

/// Longest profile request the relay reads, newline included.
pub const RELAY_REQUEST_MAX: usize = 32;

impl Profile {
    /// The profile's name on the relay socket.
    pub fn id(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::PcApp => "pc-app",
            Self::Terminal => "terminal",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [Self::Native, Self::PcApp, Self::Terminal]
            .into_iter()
            .find(|profile| profile.id() == id)
    }
}

/// The relay's request: one profile name and a newline, nothing else. The
/// relay runs with keyd's group, and keyd's IPC runs `command()` bindings as
/// root, so the session may only choose among rmac's generated profiles.
pub fn parse_relay_request(request: &[u8]) -> Option<Profile> {
    if request.len() > RELAY_REQUEST_MAX {
        return None;
    }
    let line = request.strip_suffix(b"\n")?;
    Profile::from_id(std::str::from_utf8(line).ok()?)
}

/// The members of `group` in an `/etc/group` file.
pub fn parse_group_members(source: &str, group: &str) -> Vec<String> {
    source
        .lines()
        .find_map(|line| {
            let mut fields = line.split(':');
            (fields.next()? == group).then(|| fields.nth(2).unwrap_or(""))
        })
        .map(|members| {
            members
                .split(',')
                .filter(|member| !member.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// How failures of `pkexec … rmac-mac-keyboard` are named to the user.
pub const HELPER_LABEL: &str = "the keyboard helper";

/// A short, user-facing description of a failed command. For the helper,
/// pkexec's own exit codes are told apart: 126 means the authentication
/// dialog was dismissed, 127 means authorisation was refused.
pub fn command_failure(program: &str, stderr: &[u8], code: Option<i32>) -> String {
    let detail = String::from_utf8_lossy(stderr);
    let detail = detail.trim();
    match (code, detail.is_empty()) {
        (Some(126), _) if program == HELPER_LABEL => "authentication was cancelled".into(),
        (Some(127), _) if program == HELPER_LABEL => {
            "you are not authorised to change the keyboard for all apps".into()
        }
        (_, false) => format!(
            "{program} failed: {}",
            detail.lines().last().unwrap_or(detail)
        ),
        (Some(code), true) => format!("{program} exited with status {code}"),
        (None, true) => format!("{program} was stopped by a signal"),
    }
}
