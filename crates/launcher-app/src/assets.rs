//! Spotlight glyphs traced from macOS 26.2, layered over the component icons.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "spotlight/*.svg"]
struct SpotlightAssets;

pub(crate) struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(asset) = SpotlightAssets::get(path) {
            return Ok(Some(asset.data));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = SpotlightAssets::iter()
            .filter(|asset| asset.starts_with(path))
            .map(|asset| SharedString::from(asset.to_string()))
            .collect::<Vec<_>>();
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
        ] {
            assert!(Assets.load(path).unwrap().is_some(), "{path}");
        }
    }
}
