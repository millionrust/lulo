//! Deterministic, private-safe Notes input normalization and local identifiers.

use std::collections::BTreeSet;
use std::time::SystemTime;

use gpui::SharedString;
use rmac_notes_store::{MAX_TAGS_PER_NOTE, MAX_TAG_BYTES};

pub(super) fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX - 1)
        .max(1)
}

pub(super) fn take_counter(counter: &mut u64) -> Option<u64> {
    let current = *counter;
    if current == 0 || current == u64::MAX {
        return None;
    }
    *counter = current + 1;
    Some(current)
}

pub(super) fn unique_folder_name(existing: &[String]) -> String {
    for suffix in 1..=existing.len().saturating_add(2) {
        let candidate = if suffix == 1 {
            "New Folder".to_string()
        } else {
            format!("New Folder {suffix}")
        };
        if !existing
            .iter()
            .any(|name| name == &candidate.to_lowercase())
        {
            return candidate;
        }
    }
    "Imported Notes".into()
}

pub(super) fn parse_tags(input: &str) -> Result<Vec<String>, &'static str> {
    let mut tags = Vec::new();
    let mut unique = BTreeSet::new();
    for value in input.split(',') {
        let value = value.trim().trim_start_matches('#').trim();
        if value.is_empty() {
            continue;
        }
        if value.len() > MAX_TAG_BYTES || value.chars().any(char::is_control) {
            return Err("Each Notes tag must be valid text no longer than 256 bytes");
        }
        if unique.insert(value.to_lowercase()) {
            if tags.len() == MAX_TAGS_PER_NOTE {
                return Err("A note can contain at most 32 tags");
            }
            tags.push(value.to_string());
        }
    }
    Ok(tags)
}

pub(super) fn display_title(title: &str) -> SharedString {
    if title.trim().is_empty() {
        "New Note".into()
    } else {
        title.to_string().into()
    }
}

pub(super) fn safe_export_stem(label: &str) -> String {
    const MAX_STEM_BYTES: usize = 80;
    let mut stem = String::new();
    let mut previous_space = false;
    for character in label.trim().chars() {
        let character = if character.is_control() || "/\\:*?\"<>|".contains(character) {
            '-'
        } else if character.is_whitespace() {
            ' '
        } else {
            character
        };
        if character == ' ' && previous_space {
            continue;
        }
        if stem.len().saturating_add(character.len_utf8()) > MAX_STEM_BYTES {
            break;
        }
        stem.push(character);
        previous_space = character == ' ';
    }
    let stem = stem.trim().trim_matches('.').trim();
    if stem.is_empty() {
        "Notes".into()
    } else {
        stem.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_never_wrap_or_emit_zero() {
        let mut counter = 1;
        assert_eq!(take_counter(&mut counter), Some(1));
        assert_eq!(take_counter(&mut counter), Some(2));
        counter = u64::MAX;
        assert_eq!(take_counter(&mut counter), None);
    }

    #[test]
    fn folder_names_are_case_insensitive_and_deterministic() {
        let existing = vec!["new folder".into(), "new folder 2".into()];
        assert_eq!(unique_folder_name(&existing), "New Folder 3");
    }

    #[test]
    fn empty_note_metadata_has_private_safe_fallbacks() {
        assert_eq!(display_title(""), SharedString::from("New Note"));
    }
}
