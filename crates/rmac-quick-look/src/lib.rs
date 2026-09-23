//! Quick Look for rmac, measured against macOS 26.2 (design-lab/quick-look.html).
//!
//! Space in Files (or in the Open and Save panel) opens a floating panel for
//! the selected items: images and PDFs through Preview's renderer, text and
//! code, video poster frames and audio waveforms when FFmpeg is installed,
//! and an icon with size and date for folders, archives and everything else.
//! A selection steps with ← and → (wrapping) or through the index sheet;
//! Space and Esc close, ⌥Space toggles full screen.
//!
//! Omitted, with no backend in rmac: Markup, Rotate and Share in the title
//! bar, and the zoom-from-icon animation (niri cannot place a window at a
//! point on another window's surface).

pub mod content;
pub mod metrics;
mod panel;

use std::borrow::Cow;

pub use panel::{open, Event, Handle, Options, QuickLook, APP_ID};

#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct Assets;

/// Quick Look's glyphs for the host application's `AssetSource`
/// (`icons/quick-look/…`).
pub fn asset(path: &str) -> Option<Cow<'static, [u8]>> {
    Assets::get(path).map(|file| file.data)
}

/// Asset paths under `prefix`, for `AssetSource::list`.
pub fn asset_paths(prefix: &str) -> Vec<String> {
    Assets::iter()
        .filter(|path| path.starts_with(prefix))
        .map(|path| path.into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_glyph_the_panel_draws_is_embedded() {
        for name in [
            "close",
            "full-screen",
            "chevron-left",
            "chevron-right",
            "index-sheet",
            "document",
            "folder",
        ] {
            assert!(
                super::asset(&format!("icons/quick-look/{name}.svg")).is_some(),
                "{name}"
            );
        }
    }
}
