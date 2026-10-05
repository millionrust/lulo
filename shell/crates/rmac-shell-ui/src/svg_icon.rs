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

/// `(path, rounded display size in logical px)`: the two things that
/// determine the decoded bitmap.
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

/// Draws the SVG at `path` at exactly `size` logical px — a drop-in
/// replacement for `img(path)` in a chain (`.w()`, `.h()`, `.rounded()`,
/// `.absolute()`, … all still apply to the returned builder), except the
/// bitmap is rasterized at `size`, not GPUI's default (native size × 2).
///
/// Cached process-wide per `(path, size)`. On a cache miss, the decode runs
/// on the blocking-task pool (never the UI thread); the caller gets a
/// blank placeholder for that one frame, and the entity `cx` belongs to is
/// notified to repaint once the bitmap lands.
pub fn svg_icon<T: 'static>(path: impl AsRef<Path>, size: f32, cx: &mut Context<T>) -> Img {
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
        size.max(1.0).round() as u32,
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
