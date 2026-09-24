//! macOS text-editing keys for every rmac text field and editor.
//!
//! gpui-component binds its Mac editing keys only when compiled for macOS. rmac
//! maps ⌘ to Super on Linux, so the same bindings are installed there after the
//! component defaults and take precedence over the PC set: ⌘C/⌘V/⌘Z edit,
//! ⌥←/⌥→ move by word, ⌘←/⌘→ reach the line ends, and the Emacs keys
//! (⌃A/⌃E/⌃K/⌃D/⌃H/⌃F/⌃B) edit as in Cocoa text fields instead of acting as
//! PC shortcuts.

use gpui::{App, KeyBinding};

pub(crate) fn init(cx: &mut App) {
    // macOS already receives these from gpui-component.
    if cfg!(not(target_os = "macos")) {
        bind(cx);
    }
}

fn bind(cx: &mut App) {
    use gpui_component::input::*;

    const CONTEXT: Option<&str> = Some("Input");
    cx.bind_keys([
        KeyBinding::new("cmd-backspace", DeleteToBeginningOfLine, CONTEXT),
        KeyBinding::new("cmd-delete", DeleteToEndOfLine, CONTEXT),
        KeyBinding::new("alt-backspace", DeleteToPreviousWordStart, CONTEXT),
        KeyBinding::new("alt-delete", DeleteToNextWordEnd, CONTEXT),
        KeyBinding::new("cmd-]", Indent, CONTEXT),
        KeyBinding::new("cmd-[", Outdent, CONTEXT),
        KeyBinding::new("ctrl-shift-a", SelectToStartOfLine, CONTEXT),
        KeyBinding::new("ctrl-shift-e", SelectToEndOfLine, CONTEXT),
        KeyBinding::new("shift-cmd-left", SelectToStartOfLine, CONTEXT),
        KeyBinding::new("shift-cmd-right", SelectToEndOfLine, CONTEXT),
        KeyBinding::new("alt-shift-left", SelectToPreviousWordStart, CONTEXT),
        KeyBinding::new("alt-shift-right", SelectToNextWordEnd, CONTEXT),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, CONTEXT),
        KeyBinding::new("cmd-a", SelectAll, CONTEXT),
        KeyBinding::new("cmd-c", Copy, CONTEXT),
        KeyBinding::new("cmd-x", Cut, CONTEXT),
        KeyBinding::new("cmd-v", Paste, CONTEXT),
        KeyBinding::new("ctrl-a", MoveHome, CONTEXT),
        KeyBinding::new("cmd-left", MoveHome, CONTEXT),
        KeyBinding::new("ctrl-e", MoveEnd, CONTEXT),
        // The rest of Cocoa's Emacs set: ⌃K deletes to the line's end, ⌃D
        // and ⌃H delete forward and back, ⌃F and ⌃B move by a character.
        KeyBinding::new("ctrl-k", DeleteToEndOfLine, CONTEXT),
        KeyBinding::new("ctrl-d", Delete, CONTEXT),
        KeyBinding::new("ctrl-h", Backspace, CONTEXT),
        KeyBinding::new("ctrl-f", MoveRight, CONTEXT),
        KeyBinding::new("ctrl-b", MoveLeft, CONTEXT),
        KeyBinding::new("cmd-right", MoveEnd, CONTEXT),
        KeyBinding::new("cmd-z", Undo, CONTEXT),
        KeyBinding::new("cmd-shift-z", Redo, CONTEXT),
        KeyBinding::new("cmd-up", MoveToStart, CONTEXT),
        KeyBinding::new("cmd-down", MoveToEnd, CONTEXT),
        KeyBinding::new("alt-left", MoveToPreviousWord, CONTEXT),
        KeyBinding::new("alt-right", MoveToNextWord, CONTEXT),
        KeyBinding::new("cmd-shift-up", SelectToStart, CONTEXT),
        KeyBinding::new("cmd-shift-down", SelectToEnd, CONTEXT),
        KeyBinding::new("cmd-f", Search, CONTEXT),
    ]);
}
