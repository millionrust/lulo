//! Mac shortcuts on Windows (ADR 0023), with no GPUI in sight so the menu
//! tables here and `rmac-ui`'s key bindings share one mapping.
//!
//! Windows has no free key for the Mac's ⌘: the Windows key belongs to the
//! system (Win+D, Win+L, Win+Ctrl+D…), so ⌘ is Ctrl, as in every Windows
//! app. That leaves ⌃ with no key of its own next to ⌘, so:
//!
//! - ⌘ becomes Ctrl, ⌥ Alt and ⇧ Shift (⇧⌘S → Ctrl+Shift+S).
//! - ⌃⌘ (and ⌃⌥⌘) becomes Alt+Shift: ⌃⌘N → Alt+Shift+N. ⌃⇧⌘ becomes
//!   Ctrl+Alt, and ⌃⌥⇧⌘ Ctrl+Alt+Shift. Nothing ever maps to the Windows
//!   key, and every app's menus keep distinct chords (tested below).
//! - A ⌃-letter key without ⌘ or ⌥ (⌃A, ⌃⇧E: Cocoa's Emacs keys) has no
//!   Windows form, since Ctrl+letter is a command.
//! - Anything else (arrows, ⌃Tab, plain keys) stays as it is.
//!
//! A chord Windows keeps for itself ([`WINDOWS_RESERVED`]) is never
//! produced: [`mac_to_windows`] gives `None` for it, so the app binds
//! nothing and its menu shows no hint.

/// A Windows key chord, with the key in GPUI's names (`n`, `delete`, `up`).
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Chord {
    pub win: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: String,
}

impl Chord {
    /// GPUI's spelling: `ctrl-alt-shift-n`, `win-ctrl-d`.
    pub fn unparse(&self) -> String {
        let mut parts = Vec::with_capacity(5);
        for (on, name) in [
            (self.win, "win"),
            (self.ctrl, "ctrl"),
            (self.alt, "alt"),
            (self.shift, "shift"),
        ] {
            if on {
                parts.push(name);
            }
        }
        parts.push(&self.key);
        parts.join("-")
    }

    /// As Windows menus write it: "Ctrl+Alt+Shift+N".
    pub fn display(&self) -> String {
        let mut parts = Vec::with_capacity(5);
        for (on, name) in [
            (self.win, "Win"),
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
        ] {
            if on {
                parts.push(name.to_owned());
            }
        }
        parts.push(key_display(&self.key));
        parts.join("+")
    }
}

/// Key chords Windows keeps for itself, in [`Chord::unparse`] form. Every
/// chord with the Windows key is reserved as well (Win+D, Win+L, Win+Ctrl+D
/// new desktop, Win+Ctrl+O on-screen keyboard, Win+Ctrl+N and
/// Win+Ctrl+Enter Narrator, Win+Ctrl+F4 and Win+Ctrl+←/→ desktops,
/// Win+Ctrl+C colour filters, Win+Ctrl+S speech, Win+Ctrl+Space input,
/// Win+Ctrl+Shift+B the graphics reset…); those are listed too, so the
/// table documents the ones ⌃⌘ used to collide with.
pub const WINDOWS_RESERVED: &[&str] = &[
    // Secure attention, Start and Task Manager.
    "ctrl-alt-delete",
    "ctrl-escape",
    "ctrl-shift-escape",
    // Switching windows.
    "alt-tab",
    "alt-shift-tab",
    "ctrl-alt-tab",
    "ctrl-alt-shift-tab",
    "alt-escape",
    "alt-shift-escape",
    // A window's system menu, and the input method switch of CJK keyboards.
    "alt-space",
    "ctrl-space",
    "ctrl-shift-space",
    // Screenshots and the context menu key.
    "printscreen",
    "alt-printscreen",
    "shift-f10",
    // The Windows key family ⌃⌘ used to land on.
    "win-ctrl-d",
    "win-ctrl-o",
    "win-ctrl-n",
    "win-ctrl-c",
    "win-ctrl-s",
    "win-ctrl-q",
    "win-ctrl-m",
    "win-ctrl-f",
    "win-ctrl-f4",
    "win-ctrl-left",
    "win-ctrl-right",
    "win-ctrl-enter",
    "win-ctrl-space",
    "win-ctrl-shift-b",
];

/// Whether Windows keeps `chord` for itself.
pub fn is_reserved(chord: &Chord) -> bool {
    chord.win || WINDOWS_RESERVED.contains(&chord.unparse().as_str())
}

/// The Windows chord for a Mac shortcut with these modifiers and `key`
/// (GPUI's key name), or `None` when it has none (a Cocoa ⌃-letter key, or
/// one Windows reserves).
pub fn mac_to_windows(
    command: bool,
    control: bool,
    option: bool,
    shift: bool,
    key: &str,
) -> Option<Chord> {
    if control
        && !command
        && !option
        && key.chars().count() == 1
        && key.chars().all(|c| c.is_ascii_alphabetic())
    {
        return None;
    }
    let (ctrl, alt, shift) = if command && control {
        match (option, shift) {
            (_, false) => (false, true, true),
            (false, true) => (true, true, false),
            (true, true) => (true, true, true),
        }
    } else {
        (command || control, option, shift)
    };
    let chord = Chord {
        win: false,
        ctrl,
        alt,
        shift,
        key: key.to_owned(),
    };
    (!is_reserved(&chord)).then_some(chord)
}

/// A Mac menu hint ("⌃⌘N") split into its modifiers (⌘, ⌃, ⌥, ⇧) and the
/// key in GPUI's names, or `None` for a hint with no key.
pub fn parse_hint(hint: &str) -> Option<(bool, bool, bool, bool, String)> {
    let (mut command, mut control, mut option, mut shift) = (false, false, false, false);
    let mut rest = hint;
    loop {
        let mut characters = rest.chars();
        match characters.next() {
            Some('⌘') => command = true,
            Some('⌃') => control = true,
            Some('⌥') => option = true,
            Some('⇧') => shift = true,
            _ => break,
        }
        rest = characters.as_str();
    }
    if rest.is_empty() {
        return None;
    }
    let key = match rest {
        "⌫" => "backspace".to_owned(),
        "⌦" => "delete".to_owned(),
        "↩" | "⏎" | "⌅" => "enter".to_owned(),
        "⇥" => "tab".to_owned(),
        "⎋" | "Esc" => "escape".to_owned(),
        "←" => "left".to_owned(),
        "→" => "right".to_owned(),
        "↑" => "up".to_owned(),
        "↓" => "down".to_owned(),
        "−" => "-".to_owned(),
        "⇞" => "pageup".to_owned(),
        "⇟" => "pagedown".to_owned(),
        "↖" => "home".to_owned(),
        "↘" => "end".to_owned(),
        other => other.to_lowercase(),
    };
    Some((command, control, option, shift, key))
}

/// The Windows chord a Mac menu hint stands for (see the module docs).
pub fn hint_chord(hint: &str) -> Option<Chord> {
    let (command, control, option, shift, key) = parse_hint(hint)?;
    mac_to_windows(command, control, option, shift, &key)
}

/// The Windows spelling of a Mac hint ("⌃⌘N" → "Alt+Shift+N", "⌘⌫" →
/// "Ctrl+Backspace"). A hint with no Windows chord shows nothing; one with
/// no key at all is shown as it is.
pub fn windows_hint(hint: &str) -> String {
    if parse_hint(hint).is_none() {
        return hint.to_owned();
    }
    hint_chord(hint)
        .map(|chord| chord.display())
        .unwrap_or_default()
}

fn key_display(key: &str) -> String {
    match key {
        "backspace" => "Backspace".to_owned(),
        "delete" => "Delete".to_owned(),
        "enter" => "Enter".to_owned(),
        "tab" => "Tab".to_owned(),
        "escape" => "Esc".to_owned(),
        "left" => "Left".to_owned(),
        "right" => "Right".to_owned(),
        "up" => "Up".to_owned(),
        "down" => "Down".to_owned(),
        "pageup" => "Page Up".to_owned(),
        "pagedown" => "Page Down".to_owned(),
        "home" => "Home".to_owned(),
        "end" => "End".to_owned(),
        "space" => "Space".to_owned(),
        other => {
            let mut characters = other.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect(),
                None => String::new(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_chords_read_like_windows_menus() {
        assert_eq!(windows_hint("⌘S"), "Ctrl+S");
        assert_eq!(windows_hint("⌥⇧⌘S"), "Ctrl+Alt+Shift+S");
        assert_eq!(windows_hint("⇧⌘N"), "Ctrl+Shift+N");
        assert_eq!(windows_hint("⌘⌫"), "Ctrl+Backspace");
        assert_eq!(windows_hint("⌘−"), "Ctrl+-");
        assert_eq!(windows_hint("⌘↑"), "Ctrl+Up");
        assert_eq!(windows_hint("↩"), "Enter");
        assert_eq!(windows_hint("⌦"), "Delete");
        assert_eq!(windows_hint("⇧⌦"), "Shift+Delete");
        assert_eq!(windows_hint("⌥↑"), "Alt+Up");
        assert_eq!(windows_hint("F2"), "F2");
        assert_eq!(windows_hint("Space"), "Space");
        assert_eq!(windows_hint("⌃K"), "");
        assert_eq!(windows_hint(""), "");
    }

    #[test]
    fn control_command_never_reaches_the_windows_key() {
        assert_eq!(windows_hint("⌃⌘N"), "Alt+Shift+N");
        assert_eq!(windows_hint("⌃⌘D"), "Alt+Shift+D");
        assert_eq!(windows_hint("⌃⌘O"), "Alt+Shift+O");
        assert_eq!(windows_hint("⌃⌘↑"), "Alt+Shift+Up");
        assert_eq!(windows_hint("⌃⌥⌘1"), "Alt+Shift+1");
        assert_eq!(windows_hint("⌃⇧⌘F"), "Ctrl+Alt+F");
        assert_eq!(windows_hint("⌃⌥⇧⌘F"), "Ctrl+Alt+Shift+F");
        for command in [false, true] {
            for control in [false, true] {
                for option in [false, true] {
                    for shift in [false, true] {
                        for key in ["d", "o", "n", "f4", "left", "right", "enter", "space"] {
                            if let Some(chord) =
                                mac_to_windows(command, control, option, shift, key)
                            {
                                assert!(!chord.win, "{chord:?}");
                                assert!(!is_reserved(&chord), "{chord:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn reserved_chords_are_never_produced() {
        for reserved in WINDOWS_RESERVED {
            let parts = reserved.split('-').collect::<Vec<_>>();
            let (key, modifiers) = parts.split_last().unwrap();
            let chord = Chord {
                win: modifiers.contains(&"win"),
                ctrl: modifiers.contains(&"ctrl"),
                alt: modifiers.contains(&"alt"),
                shift: modifiers.contains(&"shift"),
                key: (*key).to_owned(),
            };
            assert_eq!(chord.unparse(), *reserved, "table entries are canonical");
            assert!(is_reserved(&chord));
        }
        // ⌘Esc, ⌥Tab and ⌥Space would be Ctrl+Esc, Alt+Tab and Alt+Space.
        assert!(mac_to_windows(true, false, false, false, "escape").is_none());
        assert!(mac_to_windows(false, false, true, false, "tab").is_none());
        assert!(mac_to_windows(false, false, true, false, "space").is_none());
        assert_eq!(windows_hint("⌘Esc"), "");
        // Alt+F4 is an app's own Close and stays usable.
        assert!(mac_to_windows(false, false, true, false, "f4").is_some());
    }
}
