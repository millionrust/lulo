//! Rasterizes a master SVG icon at its actual on-screen size, cached
//! process-wide, instead of GPUI's default `img(path)` pipeline.
//!
//! Equivalent of `rmac_ui::svg_icon` for the shell bins (Dock, App
//! Switcher, Mission Control, …), which cannot depend on `rmac-ui` — that
//! crate depends on `rmac-dock`, which these bins also depend on, so the
//! other direction would be a cycle. See `rmac_ui::svg_icon`'s doc comment
//! for the full story: GPUI's `img(path)` decodes an SVG at (its own
//! `viewBox` size) × 2 regardless of display size, so Lulo's 1024×1024
//! master icons cost ~1.4–1.75s each on the reference laptop even shown at
//! a 16pt glyph. `shell/bins/rmac-wallpaper/src/linux_wayland/desktop.rs`
//! (`ICON_SVG_SCALE`, `warm_desktop_icons`) proved the fix for the
//! desktop's own folder/document glyphs; this generalizes it.
//!
//! Every shell-bin icon call site resolves to a real file on disk (Dock,
//! App Switcher and Mission Control all carry `PathBuf`s — see
//! `rmac_dock::presentation::Icon`, `rmac_apps::App::icon`), so unlike the
//! `rmac-ui` version this has no embedded-asset variant.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use gpui::{img, Context, Img, RenderImage, SharedString};

/// `(path, bucketed raster size in physical px)`: the two things that
/// determine the decoded bitmap. The raster size is `size` (the caller's
/// logical basis, which for an animated/magnified element like a Dock tile
/// should be the *maximum* it ever reaches, not its instantaneous value)
/// times the window's scale factor, rounded up to `RASTER_BUCKET` — see
/// `bucket_up`.
type CacheKey = (SharedString, u32);

struct IconCache {
    entries: HashMap<CacheKey, Arc<RenderImage>>,
    /// Insertion order for the bound below; see `rmac_ui::svg_icon`'s
    /// equivalent comment — the actual icon set is far smaller than the
    /// cap, so eviction should never trigger in practice.
    order: VecDeque<CacheKey>,
    /// Keys a background decode is already in flight for.
    pending: HashSet<CacheKey>,
}

const CACHE_CAPACITY: usize = 512;

/// Raster sizes are rounded up to the nearest multiple of this many
/// physical px before becoming a cache key. Without it, a continuously
/// changing display size (e.g. a Dock tile mid-magnification-animation)
/// would mint a fresh decode almost every frame; callers that already pass
/// a stable maximum (as the Dock now does) get one cache entry for that
/// whole size class instead. Always rounds *up*, so the rasterized bitmap
/// is never smaller than what is actually needed — GPUI only ever
/// downsamples this art, never upsamples it.
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
        })
    })
}

/// A fully transparent placeholder, decoded once, reused for every cache
/// miss's single placeholder frame instead of a visible flash or GPUI's
/// slow default decode of the real asset.
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

/// Width of the SVG's own `viewBox`, read directly out of the bytes; a
/// missing or unparsable one falls back to 1024 (this repo's convention —
/// see `rmac_ui::svg_icon::native_width`'s equivalent comment).
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

/// Draws the SVG at `path` at a logical size the *caller* sets afterwards
/// with `.w()`/`.h()`/`.size()` — a drop-in replacement for `img(path)` in
/// a chain (those, plus `.rounded()`, `.absolute()`, … all still apply to
/// the returned builder), except the bitmap is rasterized directly at the
/// display's physical size instead of GPUI's default (native size × 2).
///
/// `size` is the logical-px basis for rasterization: for an element whose
/// displayed size never changes, pass that size; for one that is
/// magnified or animated (a Dock tile under pointer magnification), pass
/// the *maximum* size it can ever reach, so the bitmap is decoded once at
/// full quality and every smaller instantaneous display size downsamples
/// from it — never upsamples. `scale_factor` is the window's
/// `Window::scale_factor()`; the actual raster target is
/// `size * scale_factor`, bucketed (see `bucket_up`) and used as the cache
/// key, so a HiDPI/fractional-scale surface (e.g. niri at 1.25) gets a
/// bitmap sized for its physical pixels, and a scale change (monitor swap,
/// settings change) naturally misses the cache and re-rasterizes.
///
/// Cached process-wide per `(path, bucketed raster px)`. On a cache miss,
/// the decode runs on the blocking-task pool (never the UI thread); the
/// caller gets a blank placeholder for that one frame, and the entity `cx`
/// belongs to is notified to repaint once the bitmap lands.
pub fn svg_icon<T: 'static>(
    path: impl AsRef<Path>,
    size: f32,
    scale_factor: f32,
    cx: &mut Context<T>,
) -> Img {
    let path = path.as_ref();
    // A `.desktop` entry's `Icon=` (first-party XDG icon paths included) is
    // format-agnostic: it may resolve to a PNG the icon theme shipped
    // instead of one of Lulo's SVG masters. Only the latter pays the
    // `img(path)` cost this module exists to avoid, and
    // `render_single_frame` cannot decode the former at all, so this stays
    // a safe drop-in by falling back to plain `img(path)` for anything
    // that isn't an SVG.
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
    {
        return img(path.to_path_buf());
    }
    let key: CacheKey = (
        SharedString::from(path.to_string_lossy().into_owned()),
        bucket_up(size.max(1.0) * scale_factor.max(1.0)),
    );
    if let Some(image) = cache().lock().unwrap().entries.get(&key).cloned() {
        return img(image);
    }
    spawn_decode(path.to_path_buf(), key, cx);
    img(blank(cx))
}

fn spawn_decode<T: 'static>(path: std::path::PathBuf, key: CacheKey, cx: &mut Context<T>) {
    {
        let mut guard = cache().lock().unwrap();
        if guard.entries.contains_key(&key) || !guard.pending.insert(key.clone()) {
            return;
        }
    }
    let renderer = cx.svg_renderer();
    let target = key.1 as f32;
    cx.spawn(async move |this, cx| {
        let decoded = blocking::unblock(move || {
            let bytes = std::fs::read(&path).ok()?;
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

#[cfg(test)]
mod tests {
    use super::bucket_up;

    /// A bucketed raster target is never smaller than what was asked for
    /// — GPUI must only ever downsample this art, never upsample it.
    #[test]
    fn bucket_up_never_rounds_down() {
        for px in [1.0_f32, 7.0, 8.0, 8.5, 20.0, 64.3, 127.999, 200.0] {
            assert!(bucket_up(px) as f32 >= px, "bucket_up({px}) rounded down");
        }
    }

    /// Niri's 1.25 scale factor on a magnified Dock icon should land on a
    /// physical-pixel bucket at least as large as the exact product, not
    /// the pre-scale logical size — this is the owner's "blurry Dock" bug.
    #[test]
    fn bucket_up_accounts_for_fractional_scale() {
        let logical_max = 80.0_f32; // a magnified Dock tile's art edge
        let scale = 1.25_f32;
        let bucketed = bucket_up(logical_max * scale);
        assert!(bucketed as f32 >= logical_max * scale);
        assert!(bucketed > logical_max.ceil() as u32);
    }

    /// Buckets collapse nearby sizes so the Dock's continuous
    /// magnification animation does not mint a fresh cache entry, and
    /// hence a fresh decode, on almost every frame.
    #[test]
    fn bucket_up_collapses_nearby_sizes() {
        assert_eq!(bucket_up(57.0), bucket_up(60.0));
        assert_eq!(bucket_up(57.0), bucket_up(64.0));
        assert_ne!(bucket_up(57.0), bucket_up(65.0));
    }
}
