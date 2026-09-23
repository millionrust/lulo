//! Pure generators: XKB options, the keyd configuration, and the per-profile
//! keyd bindings.

use rmac_locale::X11Keyboard;

use crate::{CapsLockAction, MacKeyboard, PhysicalLayout, Profile};

/// First line of every generated keyd file. It records the physical layout
/// so System Settings, the session follower and package upgrades can read the
/// file back without a second store.
pub const KEYD_HEADER_PREFIX: &str = "# rmac-mac-keyboard 1 ";

/// XKB options this feature adds and removes. Any other option (layout
/// switching, compose, …) belongs to the user and is always preserved.
pub const OWNED_XKB_OPTIONS: [&str; 7] = [
    "altwin:swap_alt_win",
    "altwin:swap_lalt_lwin",
    "ctrl:nocaps",
    "lv3:caps_switch",
    "caps:super",
    "caps:escape",
    "caps:none",
];

/// `(layout, PC variant, Mac variant)` from xkeyboard-config's
/// `rules/base.xml` (every variant whose name contains `mac`, 2026-09). The PC variant
/// is the one a Mac variant replaces and is restored when the option is
/// turned off.
const MAC_VARIANTS: [(&str, &str, &str); 31] = [
    ("ara", "", "mac"),
    ("at", "", "mac"),
    ("ch", "", "de_mac"),
    ("ch", "fr", "fr_mac"),
    ("cz", "qwerty", "qwerty-mac"),
    ("de", "", "mac"),
    ("de", "nodeadkeys", "mac_nodeadkeys"),
    ("dk", "", "mac"),
    ("dk", "nodeadkeys", "mac_nodeadkeys"),
    ("fi", "", "mac"),
    ("fr", "", "mac"),
    ("gb", "", "mac"),
    ("gb", "intl", "mac_intl"),
    ("is", "", "mac"),
    ("it", "", "mac"),
    ("jp", "", "mac"),
    ("ml", "us", "us-mac"),
    ("nl", "", "mac"),
    ("no", "", "mac"),
    ("no", "nodeadkeys", "mac_nodeadkeys"),
    ("pt", "", "mac"),
    ("pt", "nodeadkeys", "mac_nodeadkeys"),
    ("ru", "", "mac"),
    ("se", "", "mac"),
    ("ua", "", "macOS"),
    ("us", "", "mac"),
    ("us", "dvorak", "dvorak-mac"),
    // Alternatives a user may pick by hand. They count as "on", and turning
    // the option off restores the PC variant; lookups by PC variant find the
    // primary entries above first.
    ("ara", "", "mac-phonetic"),
    ("is", "", "mac_legacy"),
    ("us", "", "mac-iso"),
    ("us", "dvorak", "dvorak-mac-iso"),
];

/// The Mac variant that replaces `variant` of `layout`, if xkeyboard-config
/// has one.
pub fn mac_variant(layout: &str, variant: &str) -> Option<&'static str> {
    MAC_VARIANTS
        .iter()
        .find(|(candidate, pc, _)| *candidate == layout && *pc == variant)
        .map(|(_, _, mac)| *mac)
}

/// Whether `variant` of `layout` is one of xkeyboard-config's Mac variants.
pub fn is_mac_variant(layout: &str, variant: &str) -> bool {
    MAC_VARIANTS
        .iter()
        .any(|(candidate, _, mac)| *candidate == layout && *mac == variant)
}

fn pc_variant(layout: &str, variant: &str) -> Option<&'static str> {
    MAC_VARIANTS
        .iter()
        .find(|(candidate, _, mac)| *candidate == layout && *mac == variant)
        .map(|(_, pc, _)| *pc)
}

/// Whether ⌥ can type Mac characters with the first (primary) layout.
pub fn supports_option_characters(keyboard: &X11Keyboard) -> bool {
    let (layouts, variants) = split_layouts(keyboard);
    layouts.first().is_some_and(|layout| {
        let variant = variants.first().map(String::as_str).unwrap_or("");
        is_mac_variant(layout, variant) || mac_variant(layout, variant).is_some()
    })
}

fn split_layouts(keyboard: &X11Keyboard) -> (Vec<String>, Vec<String>) {
    let layouts = keyboard
        .layout
        .split(',')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut variants = if keyboard.variant.is_empty() {
        Vec::new()
    } else {
        keyboard
            .variant
            .split(',')
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    variants.resize(layouts.len().max(variants.len()), String::new());
    (layouts, variants)
}

fn caps_xkb_option(action: CapsLockAction) -> Option<&'static str> {
    match action {
        CapsLockAction::CapsLock => None,
        CapsLockAction::Control => Some("ctrl:nocaps"),
        // Caps Lock chooses the third level, which is what ⌥ does for
        // characters on a Mac layout. XKB has no plain "Caps as Alt".
        CapsLockAction::Option => Some("lv3:caps_switch"),
        CapsLockAction::Command => Some("caps:super"),
        CapsLockAction::Escape => Some("caps:escape"),
        CapsLockAction::NoAction => Some("caps:none"),
    }
}

/// The localed keyboard that realises `target`, preserving the layouts,
/// model and every option this feature does not own.
///
/// With Mac shortcuts on, keyd performs the swap and Caps Lock remapping
/// before XKB sees the keys, so the XKB copies are removed; applying both
/// would swap the keys twice.
pub fn xkb_for(current: &X11Keyboard, target: &MacKeyboard) -> X11Keyboard {
    let layout = target.layout;
    let (layouts, mut variants) = split_layouts(current);
    let mut right_alt_is_option = false;
    for (index, name) in layouts.iter().enumerate() {
        let variant = variants[index].clone();
        if layout.option_characters {
            if let Some(mac) = mac_variant(name, &variant) {
                variants[index] = mac.to_owned();
            }
            if index == 0 && is_mac_variant(name, &variants[index]) {
                right_alt_is_option = true;
            }
        } else if let Some(pc) = pc_variant(name, &variant) {
            variants[index] = pc.to_owned();
        }
    }

    let mut options = current
        .options
        .split(',')
        .map(str::trim)
        .filter(|option| !option.is_empty() && !OWNED_XKB_OPTIONS.contains(option))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if !target.shortcuts_in_all_apps {
        if layout.swap_command_option {
            // Mac layouts make Right Alt the ⌥ character key. Swapping only
            // the left pair keeps it; swapping both would turn it into Super.
            options.push(if right_alt_is_option {
                "altwin:swap_lalt_lwin".to_owned()
            } else {
                "altwin:swap_alt_win".to_owned()
            });
        }
        if let Some(option) = caps_xkb_option(layout.caps_lock) {
            options.push(option.to_owned());
        }
    }

    let variant = if variants.iter().all(String::is_empty) {
        String::new()
    } else {
        variants.join(",")
    };
    X11Keyboard {
        layout: current.layout.clone(),
        model: current.model.clone(),
        variant,
        options: options.join(","),
    }
}

/// Read the current state back from the keyd file (present only while Mac
/// shortcuts are on) and the localed keyboard.
pub fn detect(keyd_config: Option<&str>, keyboard: &X11Keyboard) -> MacKeyboard {
    let (layouts, variants) = split_layouts(keyboard);
    let option_characters = layouts
        .first()
        .is_some_and(|layout| is_mac_variant(layout, &variants[0]));
    if let Some(layout) = keyd_config.and_then(parse_keyd_header) {
        return MacKeyboard {
            shortcuts_in_all_apps: true,
            layout,
        };
    }
    let options = keyboard
        .options
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();
    let swap_command_option = options
        .iter()
        .any(|option| matches!(*option, "altwin:swap_alt_win" | "altwin:swap_lalt_lwin"));
    let caps_lock = CapsLockAction::ALL
        .into_iter()
        .find(|action| caps_xkb_option(*action).is_some_and(|owned| options.contains(&owned)))
        .unwrap_or_default();
    MacKeyboard {
        shortcuts_in_all_apps: false,
        layout: PhysicalLayout {
            swap_command_option,
            caps_lock,
            option_characters,
        },
    }
}

fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

/// Parse the header of a keyd file written by [`keyd_config`]. Any other
/// file (a user's own keyd configuration) yields `None`.
pub fn parse_keyd_header(source: &str) -> Option<PhysicalLayout> {
    let fields = source.lines().next()?.strip_prefix(KEYD_HEADER_PREFIX)?;
    let mut swap = None;
    let mut caps = None;
    let mut option = None;
    for field in fields.split_whitespace() {
        let (key, value) = field.split_once('=')?;
        let flag = || match value {
            "on" => Some(true),
            "off" => Some(false),
            _ => None,
        };
        match key {
            "swap" if swap.is_none() => swap = Some(flag()?),
            "caps" if caps.is_none() => caps = Some(CapsLockAction::from_id(value)?),
            "option-characters" if option.is_none() => option = Some(flag()?),
            _ => return None,
        }
    }
    Some(PhysicalLayout {
        swap_command_option: swap?,
        caps_lock: caps?,
        option_characters: option?,
    })
}

/// Keys ⌘ translates for PC apps: `(keyd key, emitted chord)`.
const PC_APP_COMMAND: [(&str, &str); 29] = [
    ("a", "C-a"),
    ("b", "C-b"),
    ("c", "C-c"),
    ("f", "C-f"),
    ("g", "C-g"),
    ("i", "C-i"),
    ("k", "C-k"),
    ("l", "C-l"),
    ("n", "C-n"),
    ("o", "C-o"),
    ("p", "C-p"),
    ("q", "C-q"),
    ("r", "C-r"),
    ("s", "C-s"),
    ("t", "C-t"),
    ("u", "C-u"),
    ("v", "C-v"),
    ("w", "C-w"),
    ("x", "C-x"),
    ("z", "C-z"),
    ("0", "C-0"),
    ("equal", "C-equal"),
    ("minus", "C-minus"),
    // ⌘[ / ⌘] go back and forward.
    ("leftbrace", "A-left"),
    ("rightbrace", "A-right"),
    // ⌘←/→ move to the line's ends, ⌘↑/↓ to the document's.
    ("left", "home"),
    ("right", "end"),
    ("up", "C-home"),
    ("down", "C-end"),
];

/// ⌥ word movement for PC apps.
const PC_APP_OPTION: [(&str, &str); 6] = [
    ("left", "C-left"),
    ("right", "C-right"),
    ("up", "C-up"),
    ("down", "C-down"),
    ("backspace", "C-backspace"),
    ("delete", "C-delete"),
];

/// PC terminals reserve Control letters for the shell, so their copy/paste
/// family sits on Control-Shift.
const TERMINAL_COMMAND: [(&str, &str); 9] = [
    ("a", "C-S-a"),
    ("c", "C-S-c"),
    ("f", "C-S-f"),
    ("n", "C-S-n"),
    ("q", "C-S-q"),
    ("t", "C-S-t"),
    ("v", "C-S-v"),
    ("w", "C-S-w"),
    ("0", "C-0"),
];

/// The Mac Terminal's ⌥ keys: ⌥←/→ send Esc-b / Esc-f and ⌥⌫ Esc-Delete,
/// the readline word motions.
const TERMINAL_OPTION: [(&str, &str); 3] = [
    ("left", "A-b"),
    ("right", "A-f"),
    ("backspace", "A-backspace"),
];

/// Native ⌥ arrows when ⌥ is the AltGr character key: rmac apps still see
/// Alt for word movement.
const NATIVE_OPTION_CHARACTER_ARROWS: [(&str, &str); 6] = [
    ("left", "A-left"),
    ("right", "A-right"),
    ("up", "A-up"),
    ("down", "A-down"),
    ("backspace", "A-backspace"),
    ("delete", "A-delete"),
];

fn command_keys(layout: &PhysicalLayout) -> [&'static str; 2] {
    if layout.swap_command_option {
        ["leftalt", "rightalt"]
    } else {
        ["leftmeta", "rightmeta"]
    }
}

fn option_keys(layout: &PhysicalLayout) -> [&'static str; 2] {
    if layout.swap_command_option {
        ["leftmeta", "rightmeta"]
    } else {
        ["leftalt", "rightalt"]
    }
}

fn caps_keyd_action(action: CapsLockAction) -> Option<&'static str> {
    match action {
        CapsLockAction::CapsLock => None,
        CapsLockAction::Control => Some("layer(control)"),
        CapsLockAction::Option => Some("layer(opt)"),
        CapsLockAction::Command => Some("layer(cmd)"),
        CapsLockAction::Escape => Some("esc"),
        CapsLockAction::NoAction => Some("noop"),
    }
}

/// The static keyd configuration installed as `/etc/keyd/rmac.conf`.
///
/// ⌘ keys activate the `cmd` layer (Super for anything unbound) and ⌥ keys
/// the `opt` layer. As installed, both layers are pass-through, so rmac apps
/// and niri see exactly what they see without keyd; the session follower
/// fills the layers per focused app with `keyd bind`, and `bind reset`
/// returns to this file.
pub fn keyd_config(layout: &PhysicalLayout) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{KEYD_HEADER_PREFIX}swap={} caps={} option-characters={}\n",
        on_off(layout.swap_command_option),
        layout.caps_lock.id(),
        on_off(layout.option_characters),
    ));
    out.push_str(
        "# Written by rmac System Settings > Keyboard; changes here are replaced.\n\
         # rmac-mac-keyboard follow fills the cmd and opt layers for the focused app.\n\n\
         [ids]\n*\n\n[main]\n",
    );
    for key in command_keys(layout) {
        out.push_str(&format!("{key} = layer(cmd)\n"));
    }
    for key in option_keys(layout) {
        out.push_str(&format!("{key} = layer(opt)\n"));
    }
    if let Some(action) = caps_keyd_action(layout.caps_lock) {
        out.push_str(&format!("capslock = {action}\n"));
    }
    out.push_str("\n[cmd:M]\n\n");
    if layout.option_characters {
        // AltGr reaches the Mac layout's third level (its ⌥ characters).
        out.push_str("[opt:G]\n");
        for (key, chord) in NATIVE_OPTION_CHARACTER_ARROWS {
            out.push_str(&format!("{key} = {chord}\n"));
        }
    } else {
        out.push_str("[opt:A]\n");
    }
    // ⌃⌘ chords belong to niri (⌃⌘Q locks, ⌃⌘F is full screen) in every
    // app, so they are passed through unchanged whatever the profile.
    out.push_str("\n[cmd+control]\n");
    for key in translated_command_keys() {
        out.push_str(&format!("{key} = C-M-{key}\n"));
    }
    out
}

/// Every key any profile translates under ⌘, in first-seen order.
fn translated_command_keys() -> Vec<&'static str> {
    let mut keys = Vec::new();
    for (key, _) in PC_APP_COMMAND.iter().chain(TERMINAL_COMMAND.iter()) {
        if !keys.contains(key) {
            keys.push(*key);
        }
    }
    keys
}

/// The arguments after `keyd bind` that switch to `profile`.
pub fn bind_arguments(profile: Profile) -> Vec<String> {
    let (command, option): (&[(&str, &str)], &[(&str, &str)]) = match profile {
        Profile::Native => (&[], &[]),
        Profile::PcApp => (&PC_APP_COMMAND, &PC_APP_OPTION),
        Profile::Terminal => (&TERMINAL_COMMAND, &TERMINAL_OPTION),
    };
    let mut arguments = vec!["reset".to_owned()];
    for (key, chord) in command {
        arguments.push(format!("cmd.{key} = {chord}"));
    }
    for (key, chord) in option {
        arguments.push(format!("opt.{key} = {chord}"));
    }
    // A Mac layout's ⌥ letters keep typing characters in every profile; only
    // the editing keys above change.
    arguments
}

/// Wayland app IDs (and XWayland classes) of PC terminal emulators, compared
/// case-insensitively.
const TERMINAL_APP_IDS: [&str; 20] = [
    "org.gnome.terminal",
    "org.gnome.ptyxis",
    "org.gnome.ptyxis.devel",
    "org.gnome.console",
    "kitty",
    "alacritty",
    "foot",
    "footclient",
    "com.mitchellh.ghostty",
    "org.wezfurlong.wezterm",
    "org.kde.konsole",
    "xterm",
    "urxvt",
    "com.raggesilver.blackbox",
    "io.elementary.terminal",
    "com.gexperts.tilix",
    "terminator",
    "rio",
    "st",
    "st-256color",
];

/// rmac's own apps read ⌘ as Super already.
const NATIVE_APP_PREFIX: &str = "org.rmac.";

/// The profile for a focused window's app ID. `None` means no window has
/// focus (the desktop, or an rmac shell surface such as Spotlight).
pub fn profile_for_app(app_id: Option<&str>) -> Profile {
    let Some(app_id) = app_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Profile::Native;
    };
    if app_id.starts_with(NATIVE_APP_PREFIX) {
        return Profile::Native;
    }
    let lower = app_id.to_ascii_lowercase();
    if TERMINAL_APP_IDS.contains(&lower.as_str()) {
        Profile::Terminal
    } else {
        Profile::PcApp
    }
}
