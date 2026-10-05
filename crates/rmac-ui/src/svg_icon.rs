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

use gpui::{img, AssetSource, Context, Img, RenderImage, SharedString};

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

/// `(source, rounded display size in logical px)`: the two things that
/// determine the decoded bitmap.
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
}

/// Generous relative to the actual working set (~100 icons × a handful of
/// sizes each), so eviction is a safety bound, not a normal occurrence.
const CACHE_CAPACITY: usize = 512;

fn cache() -> &'static Mutex<IconCache> {
    static CACHE: OnceLock<Mutex<IconCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(IconCache {
            entries: HashMap::new(),
            order: VecDeque::new(),
            pending: HashSet::new(),
        })
    })
}

/// A fully transparent placeholder, decoded once (a trivial 16×16 canvas —
/// this never costs what the real icons did) and reused for every cache
/// miss's single placeholder frame, instead of either a visible flash or
/// falling back to GPUI's slow default decode of the real asset.
fn blank<T: 'static>(cx: &mut Context<T>) -> Arc<RenderImage> {
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

/// Width of the SVG's own `viewBox`, read directly out of the bytes
/// (every master icon in `assets/icons` is `viewBox="0 0 W H"` with no
/// transform; a missing or unparsable one falls back to 1024, this repo's
/// actual convention — see the audit behind this module: ~98 SVGs with a
/// viewBox of 512 or more, nearly all exactly 1024×1024).
fn native_width(bytes: &[u8]) -> f32 {
    let text = String::from_utf8_lossy(bytes);
    text.find("viewBox=\"")
        .and_then(|start| {
            let rest = &text[start + "viewBox=\"".len()..];
            let end = rest.find('"')?;
            let mut parts = rest[..end].split_whitespace();
            parts.next()?; // min-x
            parts.next()?; // min-y
            parts.next()?.parse::<f32>().ok()
        })
        .filter(|width| *width > 0.0)
        .unwrap_or(1024.0)
}

/// Draws `source` at exactly `size` logical px — a drop-in replacement for
/// `img(source)` in a chain (`.w()`, `.h()`, `.rounded()`, `.object_fit()`,
/// `.absolute()`, … all still apply to the returned builder the same way),
/// except the bitmap is rasterized at `size`, not GPUI's default (native
/// size × 2) regardless of how the icon is shown.
///
/// Cached process-wide per `(source, size)`. On a cache miss, the decode
/// runs on the blocking-task pool (never GPUI's small `background_executor`
/// — see `desktop.rs`'s `warm_desktop_icons` — and never the UI thread);
/// the caller gets a blank placeholder for that one frame, and the entity
/// `cx` belongs to is notified to repaint once the bitmap lands.
pub fn svg_icon<T: 'static>(source: impl Into<IconSource>, size: f32, cx: &mut Context<T>) -> Img {
    let source = source.into();
    if !source.is_svg() {
        return source.fallback_img();
    }
    let key: CacheKey = (source.cache_key(), size.max(1.0).round() as u32);
    if let Some(image) = cache().lock().unwrap().entries.get(&key).cloned() {
        return img(image);
    }
    spawn_decode(source, key, cx);
    img(blank(cx))
}

fn spawn_decode<T: 'static>(source: IconSource, key: CacheKey, cx: &mut Context<T>) {
    {
        let mut guard = cache().lock().unwrap();
        if guard.entries.contains_key(&key) || !guard.pending.insert(key.clone()) {
            return;
        }
    }
    let renderer = cx.svg_renderer();
    let assets = cx.asset_source().clone();
    let target = key.1 as f32;
    cx.spawn(async move |this, cx| {
        let decoded = blocking::unblock(move || {
            let bytes = source.load_bytes(&assets)?;
            let scale = (target / native_width(&bytes)).max(0.001);
            renderer.render_single_frame(&bytes, scale).ok()
        })
        .await;
        {
            let mut guard = cache().lock().unwrap();
            guard.pending.remove(&key);
            if let Some(image) = decoded {
                guard.order.push_back(key.clone());
                guard.entries.insert(key, image);
                while guard.order.len() > CACHE_CAPACITY {
                    if let Some(oldest) = guard.order.pop_front() {
                        guard.entries.remove(&oldest);
                    }
                }
            }
        }
        let _ = this.update(cx, |_, cx| cx.notify());
    })
    .detach();
}
