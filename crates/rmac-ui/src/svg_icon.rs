//! Rasterizes a master SVG icon at its actual on-screen size, cached
//! process-wide, instead of GPUI's default `img(path)` pipeline.
//!
//! `img(path)` decodes an SVG through `SvgRenderer::render_single_frame`
//! with `scale = 1.0`, which rasterizes at (the SVG's own `viewBox` size) ×
//! 2 (GPUI's internal smoothing factor for crisp downscaling) — regardless
//! of how small the icon is actually shown. Lulo's master icons are
//! 1024×1024 artwork (some with an `feDropShadow` filter), so every one of
//! them decodes as a 2048×2048 canvas even at a 16pt menu glyph: about
//! 1.4–1.75s on the reference laptop under load, independent of which SVG.
//! `shell/bins/rmac-wallpaper/src/linux_wayland/desktop.rs`
//! (`ICON_SVG_SCALE`, `warm_desktop_icons`) proved the fix for the
//! desktop's own folder/document glyphs: rasterize once, directly, at the
//! real display size, and reuse the bitmap. This generalizes that to any
//! SVG source and size, used from the Dock, Launchpad, App Switcher,
//! Mission Control, Spotlight, notifications, Settings and Files.
//!
//! Not for raster icons (PNG/JPEG) or per-file thumbnails/previews, which
//! already decode at a bounded size — only for the shared vector app-icon
//! set `assets/icons/*.svg` and the per-app icons apps resolve to a real
//! file path.
//!
//! `shell/crates/rmac-shell-ui` carries an equivalent for the shell bins
//! (Dock, App Switcher, Mission Control, …), which cannot depend on this
//! crate (`rmac-ui` depends on `rmac-dock`, which those bins also depend
//! on — a cycle the other way around).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use gpui::{img, AssetSource, Context, Img, RenderImage, SharedString, SvgRenderer};

/// Where an icon's raw SVG bytes come from.
#[derive(Clone)]
pub enum IconSource {
    /// A real file on disk (most app icons: `rmac_apps::App::icon`,
    /// `rmac_dock`'s resolved icon paths, Finder's per-item artwork).
    Path(Arc<Path>),
    /// An embedded asset resolved through the window's `AssetSource`
    /// (`icons/*.svg` shipped in an app's own `assets/` or the shared
    /// `gpui-component-assets` crate — the same strings a plain
    /// `img("icons/…svg")` call takes today).
    Asset(SharedString),
}

impl From<PathBuf> for IconSource {
    fn from(path: PathBuf) -> Self {
        IconSource::Path(Arc::from(path.as_path()))
    }
}

impl From<&Path> for IconSource {
    fn from(path: &Path) -> Self {
        IconSource::Path(Arc::from(path))
    }
}

impl From<Arc<Path>> for IconSource {
    fn from(path: Arc<Path>) -> Self {
        IconSource::Path(path)
    }
}

impl From<SharedString> for IconSource {
    fn from(path: SharedString) -> Self {
        IconSource::Asset(path)
    }
}

impl From<&str> for IconSource {
    fn from(path: &str) -> Self {
        IconSource::Asset(SharedString::from(path.to_string()))
    }
}

impl From<String> for IconSource {
    fn from(path: String) -> Self {
        IconSource::Asset(SharedString::from(path))
    }
}

impl IconSource {
    fn cache_key(&self) -> SharedString {
        match self {
            IconSource::Path(path) => SharedString::from(path.to_string_lossy().into_owned()),
            IconSource::Asset(path) => path.clone(),
        }
    }

    /// Dock, Finder, notification and launcher icons alike can resolve to
    /// either an SVG master or a PNG the XDG icon theme shipped (a
    /// `.desktop` entry's `Icon=` is format-agnostic) — only the former
    /// pays the `img(path)` cost this module exists to avoid, and
    /// `render_single_frame` cannot decode the latter at all. A plain
    /// extension check keeps this helper a safe drop-in for every call
    /// site regardless of what actually resolves there.
    fn is_svg(&self) -> bool {
        self.cache_key().to_lowercase().ends_with(".svg")
    }

    /// Unmodified `img(source)`, for a non-SVG source this module must not
    /// touch.
    fn fallback_img(&self) -> Img {
        match self {
            IconSource::Path(path) => img(path.to_path_buf()),
            IconSource::Asset(path) => img(path.clone()),
        }
    }

    fn load_bytes(&self, assets: &Arc<dyn AssetSource>) -> Option<Vec<u8>> {
        match self {
            IconSource::Path(path) => std::fs::read(path.as_ref()).ok(),
            IconSource::Asset(path) => assets
                .load(path)
                .ok()
                .flatten()
                .map(|bytes| bytes.into_owned()),
        }
    }
}

/// `(source, bucketed raster size in physical px)`: the two things that
/// determine the decoded bitmap. The raster size is `size` (the caller's
/// logical basis — the maximum display size, for anything animated or
/// magnified) times the window's scale factor, rounded up to
/// `RASTER_BUCKET` — see `bucket_up`.
type CacheKey = (SharedString, u32);

struct IconCache {
    entries: HashMap<CacheKey, Arc<RenderImage>>,
    /// Insertion order for the bound below. The icon set actually in play
    /// (Lulo ships ~100 master SVGs, each shown at a handful of discrete
    /// sizes) is far smaller than the cap, so a precise LRU is not worth
    /// reordering this on every lookup; insertion order approximates it
    /// well enough that eviction should never trigger in practice.
    order: VecDeque<CacheKey>,
    /// Keys a background decode is already in flight for, so a burst of
    /// render passes before the first one lands doesn't queue the same
    /// decode twice.
    pending: HashSet<CacheKey>,
    /// Keys `rasterize` could not bring up to their size; drawn with GPUI's
    /// own `img(source)` instead of being retried on every frame.
    failed: HashSet<CacheKey>,
}

/// Generous relative to the actual working set (~100 icons × a handful of
/// sizes each), so eviction is a safety bound, not a normal occurrence.
const CACHE_CAPACITY: usize = 512;

/// Raster sizes are rounded up to the nearest multiple of this many
/// physical px before becoming a cache key, so the handful of distinct
/// logical sizes each icon is actually shown at (never a continuum here —
/// `rmac_shell_ui::svg_icon`'s doc comment covers the Dock's
/// continuously-animated case) collapse onto a small, bounded set of
/// bitmaps. Always rounds *up*, so GPUI only ever downsamples this art,
/// never upsamples it.
const RASTER_BUCKET: u32 = 8;

fn bucket_up(physical_px: f32) -> u32 {
    let exact = physical_px.max(1.0).ceil() as u32;
    exact.div_ceil(RASTER_BUCKET) * RASTER_BUCKET
}

fn cache() -> &'static Mutex<IconCache> {
    static CACHE: OnceLock<Mutex<IconCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(IconCache {
            entries: HashMap::new(),
            order: VecDeque::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
        })
    })
}

/// A fully transparent placeholder, decoded once (a trivial 16×16 canvas —
/// this never costs what the real icons did) and reused for every cache
/// miss's single placeholder frame, instead of either a visible flash or
/// falling back to GPUI's slow default decode of the real asset.
fn blank<T: 'static>(cx: &Context<T>) -> Arc<RenderImage> {
    static BLANK: OnceLock<Arc<RenderImage>> = OnceLock::new();
    BLANK
        .get_or_init(|| {
            const SVG: &[u8] =
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"></svg>"#;
            cx.svg_renderer()
                .render_single_frame(SVG, 1.0)
                .expect("the built-in blank placeholder SVG always decodes")
        })
        .clone()
}

/// The width the SVG renderer itself gives this SVG at scale 1.0: usvg sizes
/// a tree by the root `<svg>`'s `width` attribute when it is a plain or `px`
/// length, and only falls back to the `viewBox` width without one. Lulo's
/// installed app icons (`packaging/rmac-apps/icons`, what a `.desktop`
/// entry's `Icon=` resolves to) are 1024-unit artwork that declare
/// `width="128"`, so treating the `viewBox` as the size (as this module
/// once did) rasterized them at an eighth of the requested size — the
/// blurry Dock of build d7fa75c9. `None` when neither is readable;
/// `rasterize` then corrects from the first bitmap it gets.
fn intrinsic_width(bytes: &[u8]) -> Option<f32> {
    let text = String::from_utf8_lossy(bytes);
    let start = text.find("<svg")?;
    let tag = &text[start..start + text[start..].find('>')?];
    let attribute = |name: &str| -> Option<String> {
        let needle = format!("{name}=");
        let mut search = 0;
        while let Some(found) = tag[search..].find(&needle) {
            let at = search + found;
            search = at + needle.len();
            if !tag[..at].ends_with(char::is_whitespace) {
                continue; // e.g. `stroke-width=` when looking for `width=`
            }
            let rest = &tag[search..];
            let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
            let value = &rest[1..];
            return value.find(quote).map(|end| value[..end].trim().to_string());
        }
        None
    };
    let width = attribute("width").and_then(|value| {
        value
            .strip_suffix("px")
            .unwrap_or(&value)
            .trim()
            .parse::<f32>()
            .ok()
    });
    width
        .or_else(|| {
            let view_box = attribute("viewBox")?;
            let mut parts = view_box
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|part| !part.is_empty());
            parts.nth(2)?.parse::<f32>().ok()
        })
        .filter(|width| width.is_finite() && *width > 0.0)
}

/// The longer edge of a decoded bitmap, in pixels.
fn long_side(image: &RenderImage) -> u32 {
    let size = image.size(0);
    size.width.0.max(size.height.0).max(0) as u32
}

/// Rasterizes `bytes` so the bitmap's longer edge is at least `target`
/// physical px. The scale comes from `intrinsic_width`; the bitmap is then
/// measured, and one that still came out short (an SVG sized in units the
/// estimate does not read, such as `width="1in"`) is re-rendered with the
/// shortfall corrected. `None` when it cannot reach `target` at all, so
/// the cache never holds — and never serves — a bitmap smaller than the
/// size it is keyed by.
fn rasterize(renderer: &SvgRenderer, bytes: &[u8], target: u32) -> Option<Arc<RenderImage>> {
    let estimate = intrinsic_width(bytes).unwrap_or(1024.0);
    let mut scale = (target as f32 / estimate).max(0.001);
    for _ in 0..3 {
        let image = renderer.render_single_frame(bytes, scale).ok()?;
        let long = long_side(&image);
        if long >= target {
            return Some(image);
        }
        if long == 0 {
            return None;
        }
        scale *= (target + 1) as f32 / long as f32;
    }
    None
}

/// Draws `source` at a logical size the caller sets afterwards with
/// `.w()`/`.h()`/`.size()`/`.object_fit()` — a drop-in replacement for
/// `img(source)` in a chain (those, plus `.rounded()`, `.absolute()`, …
/// all still apply to the returned builder the same way), except the
/// bitmap is rasterized directly at the display's physical size instead of
/// GPUI's default (native size × 2) regardless of how the icon is shown.
///
/// `size` is the logical-px basis for rasterization — for a fixed-size
/// icon, its one display size; for anything whose display size varies
/// (magnified or animated), the maximum it can ever reach, so one decode
/// covers the whole range and every smaller size downsamples rather than
/// upsamples. `scale_factor` is the window's `Window::scale_factor()`; the
/// actual raster target is `size * scale_factor`, bucketed (see
/// `bucket_up`) and used as the cache key, so a HiDPI/fractional-scale
/// surface gets a bitmap sized for its physical pixels and a scale change
/// naturally misses the cache and re-rasterizes.
///
/// Cached process-wide per `(source, bucketed raster px)`. On a cache
/// miss, the decode runs on the blocking-task pool (never GPUI's small
/// `background_executor` — see `desktop.rs`'s `warm_desktop_icons` — and
/// never the UI thread); the caller gets a blank placeholder for that one
/// frame, and the entity `cx` belongs to is notified to repaint once the
/// bitmap lands.
pub fn svg_icon<T: 'static>(
    source: impl Into<IconSource>,
    size: f32,
    scale_factor: f32,
    cx: &Context<T>,
) -> Img {
    let source = source.into();
    if !source.is_svg() {
        return source.fallback_img();
    }
    let key: CacheKey = (
        source.cache_key(),
        bucket_up(size.max(1.0) * scale_factor.max(1.0)),
    );
    {
        let guard = cache().lock().unwrap();
        if let Some(image) = guard.entries.get(&key).cloned() {
            return img(image);
        }
        if guard.failed.contains(&key) {
            return source.fallback_img();
        }
    }
    spawn_decode(source, key, cx);
    img(blank(cx))
}

fn spawn_decode<T: 'static>(source: IconSource, key: CacheKey, cx: &Context<T>) {
    {
        let mut guard = cache().lock().unwrap();
        if guard.entries.contains_key(&key) || !guard.pending.insert(key.clone()) {
            return;
        }
    }
    let renderer = cx.svg_renderer();
    let assets = cx.asset_source().clone();
    let target = key.1;
    cx.spawn(async move |this, cx| {
        let decoded = blocking::unblock(move || {
            let bytes = source.load_bytes(&assets)?;
            rasterize(&renderer, &bytes, target)
        })
        .await;
        {
            let mut guard = cache().lock().unwrap();
            guard.pending.remove(&key);
            match decoded {
                None => {
                    // Never retried per frame: `svg_icon` hands this key to
                    // GPUI's own `img(source)` from now on.
                    guard.failed.insert(key);
                }
                Some(image) => {
                    guard.order.push_back(key.clone());
                    guard.entries.insert(key, image);
                    while guard.order.len() > CACHE_CAPACITY {
                        if let Some(oldest) = guard.order.pop_front() {
                            guard.entries.remove(&oldest);
                        }
                    }
                }
            }
        }
        let _ = this.update(cx, |_, cx| cx.notify());
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::{bucket_up, intrinsic_width, long_side, rasterize};
    use std::sync::Arc;

    /// An installed Lulo app icon: 1024-unit artwork declared 128 px wide.
    const FILES_ICON: &[u8] =
        include_bytes!("../../../packaging/rmac-apps/icons/org.rmac.Files.svg");
    /// A bundled master: the same artwork declared 1024 px wide.
    const NOTES_ASSET: &[u8] = include_bytes!("../../../assets/icons/notes.svg");

    /// The renderer sizes by `width`, not `viewBox`; reading the latter is
    /// what rasterized every installed app icon at an eighth of its size.
    #[test]
    fn intrinsic_width_follows_the_width_attribute() {
        assert_eq!(intrinsic_width(FILES_ICON), Some(128.0));
        assert_eq!(intrinsic_width(NOTES_ASSET), Some(1024.0));
        assert_eq!(
            intrinsic_width(br#"<svg viewBox="0,0,48,48" width="100%">"#),
            Some(48.0)
        );
        assert_eq!(intrinsic_width(b"<svg>"), None);
    }

    /// Whatever the SVG declares, the cached bitmap is at least the
    /// physical size it is keyed by.
    #[test]
    fn rasterize_never_returns_a_bitmap_smaller_than_requested() {
        let renderer = gpui::SvgRenderer::new(Arc::new(()));
        let inches = br#"<svg xmlns="http://www.w3.org/2000/svg" width="0.25in" height="0.25in" viewBox="0 0 1024 1024"><rect width="1024" height="1024" fill="red"/></svg>"#;
        for bytes in [FILES_ICON, NOTES_ASSET, inches.as_slice()] {
            for target in [16, 40, 64, 104, 160] {
                let image = rasterize(&renderer, bytes, target).expect("icon decodes");
                assert!(
                    long_side(&image) >= target,
                    "{} px bitmap for a {target} px request",
                    long_side(&image)
                );
            }
        }
    }

    /// A bucketed raster target is never smaller than what was asked for
    /// — GPUI must only ever downsample this art, never upsample it.
    #[test]
    fn bucket_up_never_rounds_down() {
        for px in [1.0_f32, 7.0, 8.0, 8.5, 20.0, 64.3, 127.999, 200.0] {
            assert!(bucket_up(px) as f32 >= px, "bucket_up({px}) rounded down");
        }
    }

    /// Niri's 1.25 scale factor on a 54 pt icon row (SET's `style::ROW_ICON`
    /// convention) should land on a physical-pixel bucket at least as large
    /// as the exact product, not the pre-scale logical size.
    #[test]
    fn bucket_up_accounts_for_fractional_scale() {
        let logical = 54.0_f32;
        let scale = 1.25_f32;
        let bucketed = bucket_up(logical * scale);
        assert!(bucketed as f32 >= logical * scale);
        // And strictly bigger than just rasterizing at the logical size —
        // the bug this module exists to fix.
        assert!(bucketed > logical.ceil() as u32);
    }

    /// Buckets collapse nearby sizes so a continuously varying display
    /// size (Dock magnification) does not mint a fresh cache entry, and
    /// hence a fresh decode, on almost every frame.
    #[test]
    fn bucket_up_collapses_nearby_sizes() {
        assert_eq!(bucket_up(57.0), bucket_up(60.0));
        assert_eq!(bucket_up(57.0), bucket_up(64.0));
        assert_ne!(bucket_up(57.0), bucket_up(65.0));
    }
}
