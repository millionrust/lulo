//! Notes' own toolbar and sidebar glyphs: original line drawings, embedded
//! and layered over the shared icons (`rmac_ui::layered_assets`).

use std::borrow::Cow;

use gpui::{px, svg, AssetSource, Hsla, Result, SharedString, Styled, Svg};

pub(super) const COMPOSE: &str = "notes/compose.svg";
pub(super) const CHECKLIST: &str = "notes/checklist.svg";
pub(super) const ATTACH: &str = "notes/attach.svg";
pub(super) const MORE: &str = "notes/more.svg";
pub(super) const SEARCH: &str = "notes/search.svg";
pub(super) const FOLDER: &str = "notes/folder.svg";
pub(super) const TRASH: &str = "notes/trash.svg";

const GLYPHS: &[(&str, &[u8])] = &[
    (COMPOSE, include_bytes!("../assets/notes/compose.svg")),
    (CHECKLIST, include_bytes!("../assets/notes/checklist.svg")),
    (ATTACH, include_bytes!("../assets/notes/attach.svg")),
    (MORE, include_bytes!("../assets/notes/more.svg")),
    (SEARCH, include_bytes!("../assets/notes/search.svg")),
    (FOLDER, include_bytes!("../assets/notes/folder.svg")),
    (TRASH, include_bytes!("../assets/notes/trash.svg")),
];

pub(super) struct NotesAssets;

impl AssetSource for NotesAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(GLYPHS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(GLYPHS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}

/// One glyph drawn at `size` in `color`.
pub(super) fn glyph(path: &'static str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(path)
        .w(px(size))
        .h(px(size))
        .flex_none()
        .text_color(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_is_embedded_and_listed() {
        for (name, bytes) in GLYPHS {
            assert!(std::str::from_utf8(bytes).unwrap().starts_with("<svg"));
            assert!(NotesAssets.load(name).unwrap().is_some());
        }
        assert_eq!(NotesAssets.list("notes/").unwrap().len(), GLYPHS.len());
        assert!(NotesAssets.load("icons/search.svg").unwrap().is_none());
    }
}
