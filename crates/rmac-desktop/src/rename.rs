//! Renaming a Desktop item, without I/O: which part of the name Finder
//! selects when editing starts, which new names are allowed, and the words
//! of Finder's rename alerts.

use std::ops::Range;

/// Longest file name, in bytes, Linux file systems accept.
pub const NAME_MAX: usize = 255;
/// Longest text treated as an extension.
const EXTENSION_MAX: usize = 15;

/// Byte offset of the dot that starts `name`'s extension, if it has one: a
/// run of letters and digits (with at least one letter) after the last dot,
/// following a non-empty stem. "21.08.12" and ".bashrc" have none.
pub fn extension_start(name: &str) -> Option<usize> {
    let dot = name.rfind('.')?;
    let extension = &name[dot + 1..];
    let valid = dot > 0
        && !extension.is_empty()
        && extension.chars().count() <= EXTENSION_MAX
        && extension.chars().all(char::is_alphanumeric)
        && extension.chars().any(char::is_alphabetic);
    valid.then_some(dot)
}

/// What Finder selects when renaming starts: the name without its
/// extension, or all of it for folders and names without one.
pub fn editable_stem(name: &str, is_directory: bool) -> Range<usize> {
    match extension_start(name).filter(|_| !is_directory) {
        Some(dot) => 0..dot,
        None => 0..name.len(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameCheck {
    /// The name did not change: nothing to do.
    Unchanged,
    /// Empty: Finder quietly keeps the old name.
    Empty,
    /// Not a usable file name ("/", NUL, "." or "..", or too long).
    Invalid,
    /// Begins with a dot, which hides the item: allowed once confirmed.
    Hidden,
    Valid,
}

pub fn check_name(old: &str, new: &str) -> NameCheck {
    if new == old {
        NameCheck::Unchanged
    } else if new.is_empty() {
        NameCheck::Empty
    } else if new == "."
        || new == ".."
        || new.contains('/')
        || new.contains('\0')
        || new.len() > NAME_MAX
    {
        NameCheck::Invalid
    } else if new.starts_with('.') {
        NameCheck::Hidden
    } else {
        NameCheck::Valid
    }
}

/// Finder's alert when another item already has the name.
pub fn taken_message(name: &str) -> String {
    format!("The name “{name}” is already taken. Please choose a different name.")
}

/// Finder's alert for a name the file system cannot hold: title and body.
pub fn invalid_message(name: &str) -> (String, &'static str) {
    (
        format!("The name “{name}” can’t be used."),
        "Try using a name with fewer characters, or with no punctuation marks.",
    )
}

/// The warning before a name that begins with a dot: title and body. The
/// buttons are Cancel and Use “.”.
pub const HIDDEN_TITLE: &str = "Are you sure you want to use a name that begins with a dot “.”?";
pub const HIDDEN_BODY: &str =
    "Names that begin with a dot are reserved for the system. The item will be hidden.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stem_is_selected_but_not_the_extension() {
        assert_eq!(editable_stem("Budget 2026.xlsx", false), 0..11);
        assert_eq!(editable_stem("archive.tar.gz", false), 0..11);
        assert_eq!(editable_stem("Projects.app", true), 0..12);
        assert_eq!(editable_stem(".bashrc", false), 0..7);
        assert_eq!(editable_stem("README", false), 0..6);
        assert_eq!(editable_stem("trailing.", false), 0..9);
        assert_eq!(
            editable_stem("Screenshot 2026-09-23 at 21.08.12", false),
            0..33
        );
        assert_eq!(editable_stem("Screenshot at 21.08.12.png", false), 0..22);
        assert_eq!(editable_stem("notes.two words", false), 0..15);
    }

    #[test]
    fn new_names_are_checked_like_finder() {
        assert_eq!(check_name("a.txt", "a.txt"), NameCheck::Unchanged);
        assert_eq!(check_name("a.txt", ""), NameCheck::Empty);
        assert_eq!(check_name("a.txt", "b/c"), NameCheck::Invalid);
        assert_eq!(check_name("a.txt", "b\0"), NameCheck::Invalid);
        assert_eq!(check_name("a.txt", "."), NameCheck::Invalid);
        assert_eq!(check_name("a.txt", ".."), NameCheck::Invalid);
        assert_eq!(check_name("a.txt", &"x".repeat(256)), NameCheck::Invalid);
        assert_eq!(check_name("a.txt", &"x".repeat(255)), NameCheck::Valid);
        assert_eq!(check_name("a.txt", ".a.txt"), NameCheck::Hidden);
        assert_eq!(check_name("a.txt", "A.txt"), NameCheck::Valid);
    }

    #[test]
    fn alerts_read_like_finder() {
        assert_eq!(
            taken_message("notes.txt"),
            "The name “notes.txt” is already taken. Please choose a different name."
        );
        assert_eq!(invalid_message("a/b").0, "The name “a/b” can’t be used.");
    }
}
