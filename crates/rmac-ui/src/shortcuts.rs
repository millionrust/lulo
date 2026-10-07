//! Shared macOS-style application shortcut vocabulary.
//!
//! `keystroke` is GPUI's portable binding form (`cmd` maps to the platform
//! command modifier). `hint` is the exact compact text shown in rmac menus.
//!
//! ## The primary modifier
//!
//! Every binding is written with `cmd`, the Mac's ⌘. On Linux and macOS that
//! is right as written: Lulo's keyd layer makes the physical key in the ⌘
//! position send Super (ADR 0017). On Windows GPUI reads `cmd` as the
//! Windows key, which Windows reserves for itself, so there the primary
//! modifier is Ctrl, as in every Windows app (ADR 0023). Bind keys through
//! [`bind_keys`] and show hints through [`display_hint`], and one definition
//! gives `ctrl-s` and "Ctrl+S" on Windows and `cmd-s` and "⌘S" elsewhere.

use std::borrow::Cow;

use gpui::{App, KeyBinding, Keystroke};

/// Whether the primary modifier is Ctrl (Windows) rather than ⌘.
pub const PRIMARY_IS_CONTROL: bool = cfg!(windows);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Shortcut {
    pub keystroke: &'static str,
    pub(crate) hint: &'static str,
}

impl Shortcut {
    /// A custom shortcut outside this module's shared vocabulary — for an
    /// app that needs a platform-specific hint of its own (Terminal's
    /// Windows Copy/Paste, which bind a different key than `cmd-c`/`cmd-v`
    /// so bare Ctrl+C/Ctrl+V keep reaching the shell; see ADR 0023).
    pub const fn new(keystroke: &'static str, hint: &'static str) -> Self {
        Self { keystroke, hint }
    }

    /// The hint as this platform shows it: "⌘S", or "Ctrl+S" on Windows.
    pub fn label(&self) -> Cow<'static, str> {
        display_hint_static(self.hint)
    }
}

/// Install `bindings`, written with `cmd` for ⌘, with this platform's
/// primary modifier (see the module documentation). Every rmac app binds
/// its keys through this rather than `App::bind_keys`.
pub fn bind_keys(cx: &mut App, bindings: impl IntoIterator<Item = KeyBinding>) {
    if PRIMARY_IS_CONTROL {
        let bindings = bindings
            .into_iter()
            .filter_map(|binding| control_primary_binding(&binding))
            .collect::<Vec<_>>();
        cx.bind_keys(bindings);
    } else {
        cx.bind_keys(bindings);
    }
}

/// `binding` with Ctrl as its primary modifier, or `None` when it is a Mac
/// ⌃-letter key (Cocoa's Emacs keys) that Ctrl now owns.
pub fn control_primary_binding(binding: &KeyBinding) -> Option<KeyBinding> {
    let source = binding
        .keystrokes()
        .iter()
        .map(|keystroke| control_primary_keystroke(keystroke.inner().clone()))
        .map(|keystroke| keystroke.map(|keystroke| keystroke.unparse()))
        .collect::<Option<Vec<_>>>()?
        .join(" ");
    KeyBinding::load(
        &source,
        binding.action().boxed_clone(),
        binding.predicate(),
        false,
        binding.action_input(),
        &gpui::DummyKeyboardMapper,
    )
    .ok()
}

/// One keystroke with Ctrl as the primary modifier:
///
/// - ⌘ becomes Ctrl (`cmd-s` → `ctrl-s`, `alt-cmd-c` → `ctrl-alt-c`).
/// - ⌃⌘ keeps both, as Win+Ctrl: Ctrl alone already means ⌘.
/// - A ⌃-letter key without ⌘ or ⌥ (⌃A, ⌃⇧E: Cocoa's Emacs keys) is
///   dropped, since Ctrl+letter is now a command.
/// - Anything else (arrows, ⌃Tab, plain keys) is unchanged.
pub fn control_primary_keystroke(mut keystroke: Keystroke) -> Option<Keystroke> {
    let modifiers = &mut keystroke.modifiers;
    if modifiers.platform && !modifiers.control {
        modifiers.platform = false;
        modifiers.control = true;
    } else if modifiers.control
        && !modifiers.platform
        && !modifiers.alt
        && keystroke.key.len() == 1
        && keystroke.key.chars().all(|c| c.is_ascii_alphabetic())
    {
        return None;
    }
    Some(keystroke)
}

/// A menu hint ("⇧⌘S") as this platform shows it: unchanged, or in the
/// Windows form ("Ctrl+Shift+S") where Ctrl is the primary modifier.
pub fn display_hint(hint: &str) -> Cow<'_, str> {
    if PRIMARY_IS_CONTROL {
        Cow::Owned(windows_hint(hint))
    } else {
        Cow::Borrowed(hint)
    }
}

fn display_hint_static(hint: &'static str) -> Cow<'static, str> {
    if PRIMARY_IS_CONTROL {
        Cow::Owned(windows_hint(hint))
    } else {
        Cow::Borrowed(hint)
    }
}

/// The Windows spelling of a Mac hint, following [`control_primary_keystroke`]:
/// modifiers in Windows' order (Win, Ctrl, Alt, Shift), joined with "+", and
/// the key glyphs as Windows names them. A ⌃-letter hint, whose binding is
/// dropped, shows nothing.
pub fn windows_hint(hint: &str) -> String {
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
        return hint.to_owned();
    }
    let key = match rest {
        "⌫" => "Backspace",
        "⌦" => "Delete",
        "↩" | "⏎" | "⌅" => "Enter",
        "⇥" => "Tab",
        "⎋" | "Esc" => "Esc",
        "←" => "Left",
        "→" => "Right",
        "↑" => "Up",
        "↓" => "Down",
        "−" => "-",
        "⇞" => "Page Up",
        "⇟" => "Page Down",
        "↖" => "Home",
        "↘" => "End",
        other => other,
    };
    if control
        && !command
        && !option
        && key.len() == 1
        && key.chars().all(|c| c.is_ascii_alphabetic())
    {
        return String::new();
    }
    let mut parts = Vec::with_capacity(5);
    if command && control {
        parts.push("Win");
    }
    if command || control {
        parts.push("Ctrl");
    }
    if option {
        parts.push("Alt");
    }
    if shift {
        parts.push("Shift");
    }
    parts.push(key);
    parts.join("+")
}

pub const NEW: Shortcut = Shortcut::new("cmd-n", "⌘N");
pub const OPEN: Shortcut = Shortcut::new("cmd-o", "⌘O");
pub const SAVE: Shortcut = Shortcut::new("cmd-s", "⌘S");
pub const SAVE_AS: Shortcut = Shortcut::new("cmd-alt-shift-s", "⌥⇧⌘S");
/// TextEdit's File ▸ Duplicate: a new window with the document's current
/// content, unsaved. Distinct from [`DUPLICATE`] (⌘D), Finder's file
/// duplicate — the Mac binds this one to the key Lulo used to bind
/// [`SAVE_AS`] to before it moved to ⌥⇧⌘S.
pub const DUPLICATE_DOCUMENT: Shortcut = Shortcut::new("cmd-shift-s", "⇧⌘S");
pub const PRINT: Shortcut = Shortcut::new("cmd-p", "⌘P");
pub const CLOSE: Shortcut = Shortcut::new("cmd-w", "⌘W");
pub const FIND: Shortcut = Shortcut::new("cmd-f", "⌘F");
pub const REPLACE: Shortcut = Shortcut::new("cmd-alt-f", "⌥⌘F");
pub const FIND_NEXT: Shortcut = Shortcut::new("cmd-g", "⌘G");
pub const FIND_PREVIOUS: Shortcut = Shortcut::new("cmd-shift-g", "⇧⌘G");
pub const SELECT_ALL: Shortcut = Shortcut::new("cmd-a", "⌘A");
pub const COPY: Shortcut = Shortcut::new("cmd-c", "⌘C");
pub const COPY_AS_PATHNAME: Shortcut = Shortcut::new("cmd-alt-c", "⌥⌘C");
pub const CUT: Shortcut = Shortcut::new("cmd-x", "⌘X");
pub const PASTE: Shortcut = Shortcut::new("cmd-v", "⌘V");
pub const UNDO: Shortcut = Shortcut::new("cmd-z", "⌘Z");
pub const NEW_TAB: Shortcut = Shortcut::new("cmd-t", "⌘T");
pub const NEXT_TAB: Shortcut = Shortcut::new("cmd-shift-]", "⇧⌘]");
pub const PREVIOUS_TAB: Shortcut = Shortcut::new("cmd-shift-[", "⇧⌘[");
pub const ZOOM_IN: Shortcut = Shortcut::new("cmd-=", "⌘=");
pub const ZOOM_IN_ALTERNATE: Shortcut = Shortcut::new("cmd-+", "⌘+");
pub const ZOOM_OUT: Shortcut = Shortcut::new("cmd--", "⌘−");
pub const ZOOM_RESET: Shortcut = Shortcut::new("cmd-0", "⌘0");
pub const BACK: Shortcut = Shortcut::new("cmd-[", "⌘[");

pub const DUPLICATE: Shortcut = Shortcut::new("cmd-d", "⌘D");
pub const DELETE: Shortcut = Shortcut::new("cmd-backspace", "⌘⌫");
pub const FORCE_DELETE: Shortcut = Shortcut::new("cmd-shift-backspace", "⇧⌘⌫");
pub const DELETE_PERMANENT: Shortcut = Shortcut::new("cmd-alt-backspace", "⌥⌘⌫");
pub const NEW_FOLDER: Shortcut = Shortcut::new("cmd-shift-n", "⇧⌘N");
pub const NEW_WINDOW: Shortcut = Shortcut::new("cmd-n", "⌘N");
pub const GO_TO_FOLDER: Shortcut = Shortcut::new("cmd-shift-g", "⇧⌘G");
pub const EMPTY_TRASH: Shortcut = Shortcut::new("cmd-shift-backspace", "⇧⌘⌫");
pub const GO_UP: Shortcut = Shortcut::new("cmd-up", "⌘↑");
pub const OPEN_SELECTION: Shortcut = Shortcut::new("cmd-down", "⌘↓");
pub const TOGGLE_HIDDEN: Shortcut = Shortcut::new("cmd-shift-.", "⇧⌘.");
pub const INFO: Shortcut = Shortcut::new("cmd-i", "⌘I");
pub const CLEAR: Shortcut = Shortcut::new("cmd-k", "⌘K");
pub const CYCLE_PROFILE: Shortcut = Shortcut::new("cmd-shift-p", "⇧⌘P");
pub const PREVIOUS_MARK: Shortcut = Shortcut::new("cmd-up", "⌘↑");
pub const NEXT_MARK: Shortcut = Shortcut::new("cmd-down", "⌘↓");
pub const SELECT_COMMAND_OUTPUT: Shortcut = Shortcut::new("cmd-shift-a", "⇧⌘A");
pub const TOGGLE_MONOSPACE: Shortcut = Shortcut::new("cmd-shift-m", "⇧⌘M");
/// Settings… in the application menu.
pub const SETTINGS: Shortcut = Shortcut::new("cmd-,", "⌘,");
/// Window and application shortcuts every rmac app answers (components.rs).
pub const MINIMIZE: Shortcut = Shortcut::new("cmd-m", "⌘M");
/// Window ▸ Zoom (⌃⌘Z): toggle the focused window between its user size
/// and the working area, as every Mac app's Window menu offers.
pub const ZOOM_WINDOW: Shortcut = Shortcut::new("ctrl-cmd-z", "⌃⌘Z");
pub const HIDE: Shortcut = Shortcut::new("cmd-h", "⌘H");
pub const HIDE_OTHERS: Shortcut = Shortcut::new("cmd-alt-h", "⌥⌘H");
pub const QUIT: Shortcut = Shortcut::new("cmd-q", "⌘Q");

pub const ENTER: Shortcut = Shortcut::new("enter", "↩");
pub const ESCAPE: Shortcut = Shortcut::new("escape", "Esc");
pub const SPACE: Shortcut = Shortcut::new("space", "Space");
/// Finder's alternate Quick Look shortcut. Space remains available there too.
pub const QUICK_LOOK: Shortcut = Shortcut::new("cmd-y", "⌘Y");
pub const LEFT: Shortcut = Shortcut::new("left", "←");
pub const RIGHT: Shortcut = Shortcut::new("right", "→");
pub const UP: Shortcut = Shortcut::new("up", "↑");
pub const DOWN: Shortcut = Shortcut::new("down", "↓");

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const ALL: &[Shortcut] = &[
        NEW,
        OPEN,
        SAVE,
        SAVE_AS,
        DUPLICATE_DOCUMENT,
        PRINT,
        CLOSE,
        FIND,
        REPLACE,
        FIND_NEXT,
        FIND_PREVIOUS,
        SELECT_ALL,
        COPY,
        COPY_AS_PATHNAME,
        CUT,
        PASTE,
        UNDO,
        NEW_TAB,
        NEXT_TAB,
        PREVIOUS_TAB,
        ZOOM_IN,
        ZOOM_IN_ALTERNATE,
        ZOOM_OUT,
        ZOOM_RESET,
        BACK,
        DUPLICATE,
        DELETE,
        FORCE_DELETE,
        DELETE_PERMANENT,
        NEW_FOLDER,
        NEW_WINDOW,
        GO_TO_FOLDER,
        EMPTY_TRASH,
        GO_UP,
        OPEN_SELECTION,
        TOGGLE_HIDDEN,
        INFO,
        CLEAR,
        CYCLE_PROFILE,
        PREVIOUS_MARK,
        NEXT_MARK,
        SELECT_COMMAND_OUTPUT,
        TOGGLE_MONOSPACE,
        SETTINGS,
        MINIMIZE,
        ZOOM_WINDOW,
        HIDE,
        HIDE_OTHERS,
        QUIT,
        ENTER,
        ESCAPE,
        SPACE,
        QUICK_LOOK,
        LEFT,
        RIGHT,
        UP,
        DOWN,
    ];

    fn windows(source: &str) -> Option<String> {
        control_primary_keystroke(gpui::Keystroke::parse(source).unwrap())
            .map(|keystroke| keystroke.unparse())
    }

    #[test]
    fn windows_uses_control_for_command() {
        assert_eq!(windows("cmd-s").as_deref(), Some("ctrl-s"));
        assert_eq!(
            windows("cmd-alt-shift-s").as_deref(),
            Some("ctrl-alt-shift-s")
        );
        assert_eq!(windows("cmd--").as_deref(), Some("ctrl--"));
        assert_eq!(windows("shift-cmd-]").as_deref(), Some("ctrl-shift-]"));
        assert_eq!(windows("cmd-backspace").as_deref(), Some("ctrl-backspace"));
        // Cocoa's Emacs keys give way to Ctrl commands.
        assert_eq!(windows("ctrl-a"), None);
        assert_eq!(windows("ctrl-shift-e"), None);
        // Keys without ⌘ stay as they are.
        assert_eq!(windows("ctrl-tab").as_deref(), Some("ctrl-tab"));
        assert_eq!(windows("alt-left").as_deref(), Some("alt-left"));
        assert_eq!(windows("escape").as_deref(), Some("escape"));
        // ⌃⌘ keeps both modifiers (Win+Ctrl).
        let both =
            control_primary_keystroke(gpui::Keystroke::parse("ctrl-cmd-s").unwrap()).unwrap();
        assert!(both.modifiers.control && both.modifiers.platform);
    }

    #[test]
    fn windows_bindings_keep_their_action_and_context() {
        let binding = gpui::KeyBinding::new("cmd-shift-z", crate::DismissMenu, Some("Notes"));
        let windows = control_primary_binding(&binding).unwrap();
        assert_eq!(windows.action().name(), "rmac_ui::DismissMenu");
        assert_eq!(windows.predicate(), binding.predicate());
        let keystroke = windows.keystrokes()[0].inner();
        assert!(keystroke.modifiers.control && keystroke.modifiers.shift);
        assert!(!keystroke.modifiers.platform);
        assert_eq!(keystroke.key, "z");
        let emacs = gpui::KeyBinding::new("ctrl-k", crate::DismissMenu, Some("Input"));
        assert!(control_primary_binding(&emacs).is_none());
    }

    #[test]
    fn windows_hints_read_like_windows_menus() {
        assert_eq!(windows_hint("⌘S"), "Ctrl+S");
        assert_eq!(windows_hint("⌥⇧⌘S"), "Ctrl+Alt+Shift+S");
        assert_eq!(windows_hint("⇧⌘N"), "Ctrl+Shift+N");
        assert_eq!(windows_hint("⌘⌫"), "Ctrl+Backspace");
        assert_eq!(windows_hint("⌘−"), "Ctrl+-");
        assert_eq!(windows_hint("⌃⌘S"), "Win+Ctrl+S");
        assert_eq!(windows_hint("⌘↑"), "Ctrl+Up");
        assert_eq!(windows_hint("↩"), "Enter");
        assert_eq!(windows_hint("Space"), "Space");
        assert_eq!(windows_hint("⌃K"), "");
        // Every shared shortcut has a Windows label.
        for shortcut in ALL {
            assert!(!windows_hint(shortcut.hint).is_empty(), "{}", shortcut.hint);
        }
    }

    #[test]
    fn shortcut_vocabulary_is_canonical_and_has_consistent_hints() {
        let mut hints = HashMap::new();
        for shortcut in ALL {
            gpui::Keystroke::parse(shortcut.keystroke)
                .unwrap_or_else(|error| panic!("invalid shortcut {}: {error}", shortcut.keystroke));
            assert!(!shortcut.keystroke.is_empty());
            assert!(!shortcut.hint.is_empty());
            assert!(!shortcut.keystroke.contains("shift-cmd"));
            assert!(!shortcut.keystroke.contains("option-cmd"));
            assert!(!shortcut.keystroke.contains("option"));
            if let Some(existing) = hints.insert(shortcut.keystroke, shortcut.hint) {
                assert_eq!(existing, shortcut.hint);
            }
        }
    }
}
