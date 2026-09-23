//! The privileged helper's command line and small system-file parsers,
//! kept pure so they are unit-tested on every platform.

use crate::{CapsLockAction, MacKeyboard, PhysicalLayout};

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
