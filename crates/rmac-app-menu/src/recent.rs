//! File ▸ Open Recent ▸, shared by every app that opens documents.
//!
//! The submenu is built fresh every time an app answers a `Layout`
//! validation request (see the crate root's doc comment): [`refresh`] finds
//! it in an already-resolved menu tree by [`menu_action`] and replaces its
//! children with up to [`MAX_ENTRIES`] documents, a separator, and
//! "Clear Menu". Reading the store at that moment is all the invalidation
//! this needs — there is nothing to poll and nothing for the app to
//! announce when the list changes.
//!
//! Rows for the individual documents and "Clear Menu" are named
//! `"{app_prefix}::OpenRecentN"` and `"{app_prefix}::ClearRecentMenu"`, where
//! `app_prefix` is the app's action namespace (`"text_editor"`, `"preview"`,
//! …) — the same prefix its other menu actions already use, not its D-Bus
//! identity. [`open_recent_index`] and [`is_clear_action`] read those names
//! back so an app never builds the strings itself.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::{Item, Menu};

/// The Mac shows at most this many recent documents per app.
pub const MAX_ENTRIES: usize = 10;

const MENU_SUFFIX: &str = "OpenRecentMenu";
const CLEAR_SUFFIX: &str = "ClearRecentMenu";
const OPEN_PREFIX: &str = "OpenRecent";

/// The action naming the Open Recent submenu itself in `app_prefix`'s menu
/// tree — never activated, only used to find the submenu to [`refresh`].
pub fn menu_action(app_prefix: &str) -> String {
    format!("{app_prefix}::{MENU_SUFFIX}")
}

/// "Clear Menu"'s action.
pub fn clear_action(app_prefix: &str) -> String {
    format!("{app_prefix}::{CLEAR_SUFFIX}")
}

/// One recent document's action, keyed by its position (0-based) in the
/// list [`build`] was given.
pub fn open_action(app_prefix: &str, index: usize) -> String {
    format!("{app_prefix}::{OPEN_PREFIX}{index}")
}

/// Whether `action` is `app_prefix`'s "Clear Menu".
pub fn is_clear_action(app_prefix: &str, action: &str) -> bool {
    action == clear_action(app_prefix)
}

/// The index `action` opens, if it is one of `app_prefix`'s Open Recent
/// rows built by [`open_action`].
pub fn open_recent_index(app_prefix: &str, action: &str) -> Option<usize> {
    action
        .strip_prefix(app_prefix)?
        .strip_prefix("::")?
        .strip_prefix(OPEN_PREFIX)?
        .parse()
        .ok()
}

/// Each path's file name, disambiguated with its parent folder's name when
/// another path shown alongside it shares that name — the Mac's own rule
/// for File ▸ Open Recent.
pub fn display_names(paths: &[PathBuf]) -> Vec<String> {
    let mut counts: HashMap<&OsStr, usize> = HashMap::new();
    for path in paths {
        if let Some(name) = path.file_name() {
            *counts.entry(name).or_insert(0) += 1;
        }
    }
    paths
        .iter()
        .map(|path| {
            let file_name = path.file_name();
            let name = file_name
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            let duplicate =
                file_name.is_some_and(|name| counts.get(name).copied().unwrap_or(0) > 1);
            if !duplicate {
                return name;
            }
            match path.parent().and_then(Path::file_name) {
                Some(parent) => format!("{name} — {}", parent.to_string_lossy()),
                None => name,
            }
        })
        .collect()
}

/// Shortens `label` to fit the wire format's item-label limit, keeping it a
/// valid label (never empty, never split mid character).
fn truncate_label(label: &str) -> String {
    // Leaves room for the trailing "…" (3 UTF-8 bytes) under the 64-byte
    // limit `crate::valid_label` enforces; kept as a literal here so this
    // module does not need that constant to be public.
    const MAX_CONTENT_BYTES: usize = 61;
    if label.len() <= MAX_CONTENT_BYTES + 3 {
        return label.to_owned();
    }
    let mut end = MAX_CONTENT_BYTES;
    while end > 0 && !label.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &label[..end])
}

/// The Open Recent submenu's children: up to [`MAX_ENTRIES`] of `paths`
/// (already filtered to documents that still exist — see
/// `rmac-recent-documents`), a separator, and "Clear Menu" — greyed out
/// while there is nothing to clear, as on the Mac.
pub fn build(app_prefix: &str, paths: &[PathBuf]) -> Vec<Item> {
    let shown = &paths[..paths.len().min(MAX_ENTRIES)];
    let mut items: Vec<Item> = display_names(shown)
        .into_iter()
        .enumerate()
        .map(|(index, name)| Item::new(truncate_label(&name), open_action(app_prefix, index), ""))
        .collect();
    let mut clear =
        Item::new("Clear Menu", clear_action(app_prefix), "").enabled(!shown.is_empty());
    clear.separator_before = true;
    items.push(clear);
    items
}

/// Replaces the Open Recent submenu's children in `menus` — the one whose
/// action is [`menu_action`]`(app_prefix)` — with fresh ones built from
/// `load_paths()`. Does nothing, and never calls `load_paths`, if `menus`
/// has no such submenu (an app that has not added one to its menu table).
pub fn refresh(menus: &mut [Menu], app_prefix: &str, load_paths: impl FnOnce() -> Vec<PathBuf>) {
    let marker = menu_action(app_prefix);
    if !menus
        .iter()
        .any(|menu| contains_action(&menu.items, &marker))
    {
        return;
    }
    let children = build(app_prefix, &load_paths());
    for menu in menus {
        if replace_children(&mut menu.items, &marker, &children) {
            return;
        }
    }
}

fn contains_action(items: &[Item], action: &str) -> bool {
    items
        .iter()
        .any(|item| item.action == action || contains_action(&item.children, action))
}

fn replace_children(items: &mut [Item], marker: &str, children: &[Item]) -> bool {
    for item in items {
        if item.action == marker {
            item.children = children.to_vec();
            return true;
        }
        if !item.children.is_empty() && replace_children(&mut item.children, marker, children) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_file_names_are_disambiguated_with_their_parent_folder() {
        let paths = [
            PathBuf::from("/home/user/Documents/notes.txt"),
            PathBuf::from("/home/user/Desktop/notes.txt"),
            PathBuf::from("/home/user/Desktop/unique.txt"),
        ];
        assert_eq!(
            display_names(&paths),
            ["notes.txt — Documents", "notes.txt — Desktop", "unique.txt",]
        );
    }

    #[test]
    fn open_action_and_index_round_trip() {
        for index in 0..MAX_ENTRIES {
            let action = open_action("text_editor", index);
            assert_eq!(open_recent_index("text_editor", &action), Some(index));
        }
        assert_eq!(
            open_recent_index("text_editor", "preview::OpenRecent0"),
            None
        );
        assert_eq!(
            open_recent_index("text_editor", "text_editor::ClearRecentMenu"),
            None
        );
    }

    #[test]
    fn clear_menu_is_disabled_when_there_is_nothing_to_clear() {
        let items = build("preview", &[]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].action, clear_action("preview"));
        assert!(!items[0].enabled);
        assert!(items[0].separator_before);
    }

    #[test]
    fn build_caps_at_max_entries_and_enables_clear_menu() {
        let paths = (0..15)
            .map(|index| PathBuf::from(format!("/home/user/file-{index}.txt")))
            .collect::<Vec<_>>();
        let items = build("text_editor", &paths);
        // MAX_ENTRIES documents plus the separator-led "Clear Menu".
        assert_eq!(items.len(), MAX_ENTRIES + 1);
        assert!(items[MAX_ENTRIES].enabled);
        assert_eq!(items[0].action, open_action("text_editor", 0));
        assert_eq!(
            items[MAX_ENTRIES - 1].action,
            open_action("text_editor", MAX_ENTRIES - 1)
        );
    }

    #[test]
    fn labels_never_exceed_the_wire_limit() {
        let long_name = "x".repeat(200) + ".txt";
        let items = build(
            "preview",
            &[PathBuf::from(format!("/home/user/{long_name}"))],
        );
        assert!(items[0].label.len() <= 64);
        assert!(!items[0].label.is_empty());
    }

    #[test]
    fn refresh_replaces_only_the_matching_submenus_children() {
        let mut menus = vec![Menu {
            label: "File".into(),
            items: vec![Item::submenu(
                "Open Recent",
                menu_action("text_editor"),
                vec![Item::new("Clear Menu", clear_action("text_editor"), "")],
            )],
        }];
        let mut called = false;
        refresh(&mut menus, "text_editor", || {
            called = true;
            vec![PathBuf::from("/home/user/document.txt")]
        });
        assert!(called);
        let children = &menus[0].items[0].children;
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].action, open_action("text_editor", 0));
        assert_eq!(children[1].action, clear_action("text_editor"));
    }

    #[test]
    fn refresh_is_a_no_op_and_never_reads_the_store_without_a_submenu() {
        let mut menus = vec![Menu {
            label: "File".into(),
            items: vec![Item::new("Open…", "text_editor::OpenFile", "")],
        }];
        let before = menus.clone();
        refresh(&mut menus, "text_editor", || {
            panic!("load_paths must not run without a matching submenu");
        });
        assert_eq!(menus, before);
    }
}
