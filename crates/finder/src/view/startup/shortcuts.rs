use super::*;

pub(super) fn bind_finder_keys(cx: &mut Context<FinderView>) {
    // Every Files window shares one process and one keymap; bind once.
    static BOUND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if BOUND.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    // Keyboard shortcuts → actions (handled on the focused list), written
    // with ⌘ and bound with Ctrl in its place on Windows.
    let mut bindings = vec![
        KeyBinding::new("tab", RenameNextItem, Some("FinderRename > Input")),
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
        KeyBinding::new("alt-shift-cmd-d", DuplicateExactly, Some("Finder")),
        KeyBinding::new("cmd-e", Eject, Some("Finder")),
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
        KeyBinding::new("ctrl-cmd-up", GoUpInNewWindow, Some("Finder")),
        KeyBinding::new("cmd-[", GoBack, Some("Finder")),
        KeyBinding::new("cmd-]", GoForward, Some("Finder")),
        KeyBinding::new("cmd-shift-h", GoHome, Some("Finder")),
        KeyBinding::new("cmd-shift-a", GoApplications, Some("Finder")),
        KeyBinding::new("cmd-shift-u", GoUtilities, Some("Finder")),
        KeyBinding::new("cmd-shift-s", GoShared, Some("Finder")),
        KeyBinding::new("cmd-alt-l", GoDownloads, Some("Finder")),
        KeyBinding::new("cmd-1", ViewAsIcons, Some("Finder")),
        KeyBinding::new("cmd-2", ViewAsList, Some("Finder")),
        KeyBinding::new("cmd-3", ViewAsColumns, Some("Finder")),
        KeyBinding::new("cmd-4", ViewAsGallery, Some("Finder")),
        KeyBinding::new("ctrl-cmd-0", UseGroups, Some("Finder")),
        KeyBinding::new("ctrl-alt-cmd-1", SortByName, Some("Finder")),
        KeyBinding::new("ctrl-alt-cmd-2", SortByKind, Some("Finder")),
        KeyBinding::new("ctrl-alt-cmd-3", SortByLastOpened, Some("Finder")),
        KeyBinding::new("ctrl-alt-cmd-4", SortByAdded, Some("Finder")),
        KeyBinding::new("ctrl-alt-cmd-5", SortByDate, Some("Finder")),
        KeyBinding::new("ctrl-alt-cmd-6", SortBySize, Some("Finder")),
        KeyBinding::new("ctrl-alt-cmd-7", SortByTags, Some("Finder")),
        KeyBinding::new("alt-cmd-1", CleanUpByName, Some("Finder")),
        KeyBinding::new("alt-cmd-2", CleanUpByKind, Some("Finder")),
        KeyBinding::new("alt-cmd-5", CleanUpByDate, Some("Finder")),
        KeyBinding::new("alt-cmd-6", CleanUpBySize, Some("Finder")),
        KeyBinding::new("alt-cmd-7", CleanUpByTags, Some("Finder")),
        KeyBinding::new("cmd-shift-\\", ShowAllTabs, Some("Finder")),
        KeyBinding::new("cmd-j", ShowViewOptions, Some("Finder")),
        KeyBinding::new("cmd-shift-p", TogglePreview, Some("Finder")),
        KeyBinding::new(
            rmac_ui::shortcuts::OPEN_SELECTION.keystroke,
            OpenItems,
            Some("Finder"),
        ),
        // Return renames on the Mac; Enter opens in Explorer, where F2
        // renames (below).
        if WINDOWS_KEYS {
            KeyBinding::new(
                rmac_ui::shortcuts::ENTER.keystroke,
                OpenItems,
                Some("Finder"),
            )
        } else {
            KeyBinding::new(
                rmac_ui::shortcuts::ENTER.keystroke,
                RenameItem,
                Some("Finder"),
            )
        },
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
        KeyBinding::new(
            rmac_ui::shortcuts::QUICK_LOOK.keystroke,
            QuickLook,
            Some("Finder"),
        ),
        KeyBinding::new("alt-cmd-y", Slideshow, Some("Finder")),
        KeyBinding::new(rmac_ui::shortcuts::INFO.keystroke, GetInfo, Some("Finder")),
        KeyBinding::new("alt-cmd-i", ShowInspector, Some("Finder")),
        KeyBinding::new("ctrl-cmd-i", GetSummaryInfo, Some("Finder")),
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
        KeyBinding::new("alt-cmd-w", CloseAll, Some("Finder")),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("Finder")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Finder")),
        KeyBinding::new("cmd-shift-[", PreviousTab, Some("Finder")),
        KeyBinding::new("cmd-shift-]", NextTab, Some("Finder")),
        // View ▸ Hide Sidebar, View ▸ Show Path Bar and Go ▸ Computer.
        KeyBinding::new("ctrl-cmd-s", ToggleSidebar, Some("Finder")),
        KeyBinding::new("alt-cmd-p", TogglePathBar, Some("Finder")),
        KeyBinding::new("cmd-/", ToggleStatusBar, Some("Finder")),
        KeyBinding::new("cmd-shift-t", ToggleTabBar, Some("Finder")),
        KeyBinding::new("alt-cmd-t", ToggleToolbar, Some("Finder")),
        KeyBinding::new("cmd-l", MakeAlias, Some("Finder")),
        KeyBinding::new("ctrl-cmd-n", NewFolderWithSelection, Some("Finder")),
        KeyBinding::new("cmd-r", ShowOriginal, Some("Finder")),
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
        KeyBinding::new("ctrl-shift-cmd-f", FindByName, Some("Finder")),
        KeyBinding::new("alt-cmd-c", CopyAsPathname, Some("Finder")),
        KeyBinding::new("ctrl-alt-cmd-c", CopyAsLink, Some("Finder")),
        KeyBinding::new("alt-cmd-a", DeselectAll, Some("Finder")),
        KeyBinding::new("ctrl-cmd-o", OpenSelectionInNewTab, Some("Finder")),
        KeyBinding::new(
            "alt-cmd-o",
            OpenSelectionInNewWindowAndClose,
            Some("Finder"),
        ),
        KeyBinding::new(
            "alt-shift-cmd-backspace",
            EmptyTrashImmediately,
            Some("Finder"),
        ),
        KeyBinding::new("ctrl-cmd-t", AddToSidebar, Some("Finder")),
        KeyBinding::new("alt-cmd-v", MoveItemHere, Some("Finder")),
        KeyBinding::new("alt-shift-cmd-v", PasteExactly, Some("Finder")),
        KeyBinding::new("cmd-shift-d", GoDesktop, Some("Finder")),
        KeyBinding::new("cmd-shift-o", GoDocuments, Some("Finder")),
        KeyBinding::new("cmd-shift-f", GoRecents, Some("Finder")),
        // Finder ▸ Settings… ⌘,
        KeyBinding::new(
            rmac_ui::shortcuts::SETTINGS.keystroke,
            ShowSettings,
            Some("Finder"),
        ),
    ];
    if WINDOWS_KEYS {
        bindings.extend(windows_native_bindings());
    }
    rmac_ui::shortcuts::bind_keys(cx, bindings);
}

/// Explorer's own keys, which Windows users expect from a file browser.
const WINDOWS_KEYS: bool = rmac_ui::shortcuts::PRIMARY_IS_CONTROL;

/// Explorer's keys on Windows, beside the Mac ones (ADR 0023): Delete moves
/// to the Recycle Bin, Shift+Delete deletes immediately (which asks first),
/// F2 renames, Alt+↑ goes to the enclosing folder, and Alt+←/→ and
/// Backspace go back and forward. A rename field or the search field takes
/// its own Delete and Backspace first: their context is deeper.
fn windows_native_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("delete", MoveToTrash, Some("Finder")),
        KeyBinding::new("shift-delete", DeletePermanently, Some("Finder")),
        KeyBinding::new("f2", RenameItem, Some("Finder")),
        KeyBinding::new("alt-up", GoUp, Some("Finder")),
        KeyBinding::new("alt-left", GoBack, Some("Finder")),
        KeyBinding::new("alt-right", GoForward, Some("Finder")),
        KeyBinding::new("backspace", GoBack, Some("Finder")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_keys_reach_their_file_commands() {
        let names = windows_native_bindings()
            .iter()
            .map(|binding| {
                (
                    binding.keystrokes()[0].inner().unparse(),
                    binding.action().name(),
                )
            })
            .collect::<Vec<_>>();
        for (key, action) in [
            ("delete", "finder::MoveToTrash"),
            ("shift-delete", "finder::DeletePermanently"),
            ("f2", "finder::RenameItem"),
            ("alt-up", "finder::GoUp"),
            ("backspace", "finder::GoBack"),
        ] {
            assert!(
                names.iter().any(|(k, a)| k == key && *a == action),
                "{key} -> {action}: {names:?}"
            );
        }
        // None of them is a chord Windows keeps for itself.
        for binding in windows_native_bindings() {
            assert!(!rmac_ui::shortcuts::is_reserved_on_windows(
                binding.keystrokes()[0].inner()
            ));
        }
    }
}
