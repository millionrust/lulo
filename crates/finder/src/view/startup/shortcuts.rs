use super::*;

pub(super) fn bind_finder_keys(cx: &mut Context<FinderView>) {
    // Every Files window shares one process and one keymap; bind once.
    static BOUND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if BOUND.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    // Keyboard shortcuts → actions (handled on the focused list).
    cx.bind_keys([
        KeyBinding::new(
            rmac_ui::shortcuts::SELECT_ALL.keystroke,
            SelectAll,
            Some("Finder"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::COPY.keystroke,
            CopyItems,
            Some("Finder"),
        ),
        KeyBinding::new(rmac_ui::shortcuts::CUT.keystroke, CutItems, Some("Finder")),
        KeyBinding::new(
            rmac_ui::shortcuts::PASTE.keystroke,
            PasteItems,
            Some("Finder"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::UNDO.keystroke,
            UndoOperation,
            Some("Finder"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::DUPLICATE.keystroke,
            Duplicate,
            Some("Finder"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::DELETE.keystroke,
            MoveToTrash,
            Some("Finder"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::DELETE_PERMANENT.keystroke,
            DeletePermanently,
            Some("Finder"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::NEW_FOLDER.keystroke,
            NewFolder,
            Some("Finder"),
        ),
        KeyBinding::new(rmac_ui::shortcuts::GO_UP.keystroke, GoUp, Some("Finder")),
        KeyBinding::new("cmd-[", GoBack, Some("Finder")),
        KeyBinding::new("cmd-]", GoForward, Some("Finder")),
        KeyBinding::new("cmd-shift-h", GoHome, Some("Finder")),
        KeyBinding::new("cmd-shift-a", GoApplications, Some("Finder")),
        KeyBinding::new("cmd-alt-l", GoDownloads, Some("Finder")),
        KeyBinding::new("cmd-1", ViewAsIcons, Some("Finder")),
        KeyBinding::new("cmd-2", ViewAsList, Some("Finder")),
        KeyBinding::new("cmd-3", ViewAsColumns, Some("Finder")),
        KeyBinding::new("cmd-4", ViewAsGallery, Some("Finder")),
        KeyBinding::new(
            rmac_ui::shortcuts::OPEN_SELECTION.keystroke,
            OpenItems,
            Some("Finder"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::ENTER.keystroke,
            RenameItem,
            Some("Finder"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::TOGGLE_HIDDEN.keystroke,
            ToggleHidden,
            Some("Finder"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::SPACE.keystroke,
            QuickLook,
            Some("Finder"),
        ),
        KeyBinding::new(rmac_ui::shortcuts::INFO.keystroke, GetInfo, Some("Finder")),
        KeyBinding::new(
            rmac_ui::shortcuts::NEW_TAB.keystroke,
            NewTab,
            Some("Finder"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::CLOSE.keystroke,
            CloseTab,
            Some("Finder"),
        ),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("Finder")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Finder")),
        // View ▸ Hide Sidebar, View ▸ Show Path Bar and Go ▸ Computer.
        KeyBinding::new("ctrl-cmd-s", ToggleSidebar, Some("Finder")),
        KeyBinding::new("alt-cmd-p", TogglePathBar, Some("Finder")),
        KeyBinding::new("cmd-shift-c", GoComputer, Some("Finder")),
        KeyBinding::new(
            rmac_ui::shortcuts::NEW_WINDOW.keystroke,
            NewWindow,
            Some("Finder"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::GO_TO_FOLDER.keystroke,
            GoToFolder,
            Some("Finder"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::EMPTY_TRASH.keystroke,
            EmptyTrash,
            Some("Finder"),
        ),
        // File ▸ Open ⌘O: the same open behaviour as ⌘↓, Finder's other
        // shortcut for it.
        KeyBinding::new(
            rmac_ui::shortcuts::OPEN.keystroke,
            OpenItems,
            Some("Finder"),
        ),
        KeyBinding::new(rmac_ui::shortcuts::FIND.keystroke, Find, Some("Finder")),
        KeyBinding::new("alt-cmd-c", CopyAsPathname, Some("Finder")),
        KeyBinding::new("alt-cmd-v", MoveItemHere, Some("Finder")),
        KeyBinding::new("cmd-shift-d", GoDesktop, Some("Finder")),
        KeyBinding::new("cmd-shift-o", GoDocuments, Some("Finder")),
        KeyBinding::new("cmd-shift-f", GoRecents, Some("Finder")),
    ]);
}
