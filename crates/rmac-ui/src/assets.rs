//! App assets layered over the shared component icon set, so apps can ship
//! their own SVGs without naming the component library's asset crate.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// An app's own [`AssetSource`] consulted first, then the shared icons
/// (`icons/*.svg`) that rmac-ui's controls draw.
pub struct LayeredAssets<A> {
    app: A,
}

/// Layer `app` over the shared icons; pass the result to
/// [`crate::boot_app_with_assets`].
pub fn layered_assets<A: AssetSource>(app: A) -> LayeredAssets<A> {
    LayeredAssets { app }
}

impl<A: AssetSource> AssetSource for LayeredAssets<A> {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(asset) = self.app.load(path)? {
            return Ok(Some(asset));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = self.app.list(path)?;
        if let Ok(mut shared) = gpui_component_assets::Assets.list(path) {
            assets.append(&mut shared);
        }
        Ok(assets)
    }
}
