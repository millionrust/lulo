//! Spotlight glyphs traced from macOS 26.2, layered over the component icons.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "spotlight/*.svg"]
struct SpotlightAssets;

/// The rmac app icons Spotlight shows beside answers: Calculator for sums
/// and conversions, Clock for the time in a city, Files for "Search in
/// Files". The same files the desktop entries install.
#[derive(rust_embed::RustEmbed)]
#[folder = "../../packaging/rmac-apps/icons"]
#[prefix = "spotlight/apps/"]
#[include = "org.rmac.Calculator.svg"]
#[include = "org.rmac.Clock.svg"]
#[include = "org.rmac.Files.svg"]
struct AppIcons;

pub(crate) struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(asset) = SpotlightAssets::get(path) {
            return Ok(Some(asset.data));
        }
        if let Some(asset) = AppIcons::get(path) {
            return Ok(Some(asset.data));
        }
        // Quick Look (⌘Y) opens from this process and draws its own glyphs.
        if let Some(asset) = rmac_quick_look::asset(path) {
            return Ok(Some(asset));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = SpotlightAssets::iter()
            .chain(AppIcons::iter())
            .filter(|asset| asset.starts_with(path))
            .map(|asset| SharedString::from(asset.to_string()))
            .collect::<Vec<_>>();
        assets.extend(
            rmac_quick_look::asset_paths(path)
                .into_iter()
                .map(SharedString::from),
        );
        if let Ok(mut component_assets) = gpui_component_assets::Assets.list(path) {
            assets.append(&mut component_assets);
        }
        Ok(assets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spotlight_glyphs_are_embedded() {
        for path in [
            "spotlight/search.svg",
            "spotlight/apps.svg",
            "spotlight/folder.svg",
            "spotlight/shortcuts.svg",
            "spotlight/clipboard.svg",
            "spotlight/copy.svg",
            "spotlight/folder-inline.svg",
            "spotlight/apps/org.rmac.Calculator.svg",
            "spotlight/apps/org.rmac.Clock.svg",
            "spotlight/apps/org.rmac.Files.svg",
            "icons/quick-look/close.svg",
        ] {
            assert!(Assets.load(path).unwrap().is_some(), "{path}");
        }
    }
}
