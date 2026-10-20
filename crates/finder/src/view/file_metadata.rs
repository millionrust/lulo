//! Finder metadata stored beside a file's contents: the colour tag's display
//! form and swatch, and the Spotlight-style comment (Get Info ▸ Comments,
//! the list view's Comments column).
//!
//! Linux keeps the comment in the freedesktop.org `user.xdg.comment`
//! attribute, which other file managers read too; Windows keeps it in an
//! NTFS alternate data stream beside the tag stream (ADR 0023 phase 4).

use super::item_operations::{clear_file_tag, read_file_tag, write_file_tag, FILE_TAGS};
use super::*;

/// Finder's display name for a stored tag ("gray" → "Grey" in en-GB, as
/// the sidebar and Settings ▸ Tags spell it).
pub(super) fn tag_display_name(tag: &str) -> String {
    if tag.eq_ignore_ascii_case("gray") {
        return "Grey".to_owned();
    }
    let mut chars = tag.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The swatch colour of a stored tag, from the theme's system colours.
pub(super) fn tag_colour(tag: &str) -> Hsla {
    let colors = rmac_ui::theme::current().colors;
    match tag {
        "red" => colors.system_red.hsla(),
        "orange" => colors.system_orange.hsla(),
        "yellow" => colors.system_yellow.hsla(),
        "green" => colors.system_green.hsla(),
        "blue" => colors.system_blue.hsla(),
        "purple" => colors.system_purple.hsla(),
        _ => colors.system_gray.hsla(),
    }
}

/// Every Finder colour tag, as stored, in the Mac's order.
pub(super) fn file_tags() -> &'static [&'static str] {
    &FILE_TAGS
}

/// Set (`Some`) or clear the one colour tag on `path`, keeping the Linux tag
/// index in step, as the context menu's tag dots do.
pub(super) fn apply_file_tag(path: &Path, tag: Option<&'static str>) -> Result<(), String> {
    let result = match tag {
        Some(tag) => write_file_tag(path, tag).map_err(|error| error.to_string()),
        None if read_file_tag(path).is_err() => Ok(()),
        None => clear_file_tag(path).map_err(|error| error.to_string()),
    };
    #[cfg(any(target_os = "linux", all(test, unix)))]
    if result.is_ok() {
        rmac_search::tag_index::record(path, tag);
    }
    result
}

#[cfg(target_os = "macos")]
const FILE_COMMENT_XATTR: &str = "com.rmac.comment";
#[cfg(not(any(target_os = "macos", windows)))]
const FILE_COMMENT_XATTR: &str = "user.xdg.comment";

/// A cap that keeps one oversized attribute from costing every listing.
pub(super) const MAX_COMMENT_BYTES: usize = 4096;

fn comment_text(bytes: &[u8]) -> Option<SharedString> {
    let bytes = &bytes[..bytes.len().min(MAX_COMMENT_BYTES)];
    let text = String::from_utf8_lossy(bytes).trim().to_owned();
    (!text.is_empty()).then(|| text.into())
}

#[cfg(not(windows))]
pub(super) fn read_file_comment(path: &Path) -> Option<SharedString> {
    let mut buffer = vec![0u8; MAX_COMMENT_BYTES];
    let length = rustix::fs::getxattr(path, FILE_COMMENT_XATTR, &mut buffer).ok()?;
    comment_text(&buffer[..length])
}

#[cfg(windows)]
fn comment_stream_path(path: &Path) -> PathBuf {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":lulo.comment");
    PathBuf::from(stream)
}

#[cfg(windows)]
pub(super) fn read_file_comment(path: &Path) -> Option<SharedString> {
    comment_text(&std::fs::read(comment_stream_path(path)).ok()?)
}

/// The comment as Finder stores it: trimmed and capped on a character
/// boundary.
fn stored_comment(comment: &str) -> &str {
    let comment = comment.trim();
    let mut end = comment.len().min(MAX_COMMENT_BYTES);
    while !comment.is_char_boundary(end) {
        end -= 1;
    }
    &comment[..end]
}

/// Store `comment` on `path`; an empty comment removes it.
pub(super) fn write_file_comment(path: &Path, comment: &str) -> Result<(), String> {
    let comment = stored_comment(comment);
    #[cfg(not(windows))]
    {
        if comment.is_empty() {
            return match rustix::fs::removexattr(path, FILE_COMMENT_XATTR) {
                Ok(()) => Ok(()),
                Err(_) if read_file_comment(path).is_none() => Ok(()),
                Err(error) => Err(error.to_string()),
            };
        }
        rustix::fs::setxattr(
            path,
            FILE_COMMENT_XATTR,
            comment.as_bytes(),
            rustix::fs::XattrFlags::empty(),
        )
        .map_err(|error| error.to_string())
    }
    #[cfg(windows)]
    {
        let stream = comment_stream_path(path);
        if comment.is_empty() {
            return match std::fs::remove_file(stream) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.to_string()),
            };
        }
        std::fs::write(stream, comment.as_bytes()).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_names_are_spelt_as_finder_shows_them() {
        assert_eq!(tag_display_name("blue"), "Blue");
        assert_eq!(tag_display_name("gray"), "Grey");
        assert_eq!(tag_display_name(""), "");
        assert_eq!(file_tags().len(), 7);
    }

    #[test]
    fn comments_are_trimmed_and_capped_on_a_character_boundary() {
        assert_eq!(stored_comment("  notes \n"), "notes");
        let long = "é".repeat(MAX_COMMENT_BYTES);
        let stored = stored_comment(&long);
        assert!(stored.len() <= MAX_COMMENT_BYTES);
        assert!(stored.chars().all(|c| c == 'é'));
        assert_eq!(comment_text(b"  \n"), None);
    }

    #[test]
    fn comments_round_trip_and_clear() {
        let path =
            std::env::temp_dir().join(format!("rmac-comment-test-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"body").unwrap();
        if write_file_comment(&path, "  Quarterly figures  ").is_err() {
            // A filesystem without user attributes: nothing to verify.
            let _ = std::fs::remove_file(path);
            return;
        }
        assert_eq!(
            read_file_comment(&path).as_deref(),
            Some("Quarterly figures")
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"body");
        write_file_comment(&path, "").unwrap();
        assert_eq!(read_file_comment(&path), None);
        write_file_comment(&path, "").unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_tag_can_be_set_and_cleared_without_one_present() {
        let path = std::env::temp_dir().join(format!("rmac-tag-apply-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"body").unwrap();
        assert!(apply_file_tag(&path, None).is_ok());
        if apply_file_tag(&path, Some("green")).is_ok() {
            assert_eq!(
                super::super::item_operations::file_tag_label(&path).as_deref(),
                Some("green")
            );
            assert!(apply_file_tag(&path, None).is_ok());
            assert_eq!(super::super::item_operations::file_tag_label(&path), None);
        }
        let _ = std::fs::remove_file(path);
    }
}
