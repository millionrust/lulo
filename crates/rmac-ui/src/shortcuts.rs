//! Shared macOS-style application shortcut vocabulary.
//!
//! `keystroke` is GPUI's portable binding form (`cmd` maps to the platform
//! command modifier). `hint` is the exact compact text shown in rmac menus.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Shortcut {
    pub keystroke: &'static str,
    pub(crate) hint: &'static str,
}

impl Shortcut {
    const fn new(keystroke: &'static str, hint: &'static str) -> Self {
        Self { keystroke, hint }
    }
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
pub const HIDE: Shortcut = Shortcut::new("cmd-h", "⌘H");
pub const HIDE_OTHERS: Shortcut = Shortcut::new("cmd-alt-h", "⌥⌘H");
pub const QUIT: Shortcut = Shortcut::new("cmd-q", "⌘Q");

pub const ENTER: Shortcut = Shortcut::new("enter", "↩");
pub const ESCAPE: Shortcut = Shortcut::new("escape", "Esc");
pub const SPACE: Shortcut = Shortcut::new("space", "Space");
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
        HIDE,
        HIDE_OTHERS,
        QUIT,
        ENTER,
        ESCAPE,
        SPACE,
        LEFT,
        RIGHT,
        UP,
        DOWN,
    ];

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
