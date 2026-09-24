//! LOCK-01: reads the two small, fixed-size raw pixel caches a trusted,
//! already-image-capable *other* process (the resident wallpaper renderer,
//! `shell/bins/rmac-wallpaper`) writes for this lock screen — the current
//! wallpaper, blurred and shrunk, and the account picture.
//!
//! This process holds the user's typed password in memory
//! (`crates/rmac-lock-provider-linux/src/secret.rs`) and its accepted
//! dependency surface is deliberately narrow
//! (`docs/rmac-lock-provider-dependency-review.md`), so it must never decode
//! an arbitrary image format itself: a bug in a PNG/JPEG decoder fed a
//! malicious wallpaper or account picture file would run inside the same
//! process as that secret. Instead it reads a file of *exactly* the
//! expected byte length — no header, no format, nothing to parse — and
//! rejects anything else outright, falling back to the existing procedural
//! Aurora gradient and monogram disc exactly as before.

use std::fs;
use std::path::PathBuf;

use crate::paint::PictureRaster;

/// Kept equal to `rmac_wallpaper_image::LOCK_THUMBNAIL_WIDTH` /
/// `_HEIGHT` by convention rather than by dependency: this crate does not
/// depend on the wallpaper image crate (see the module doc above), so the
/// two must be kept in sync by hand if either changes.
const BACKGROUND_WIDTH: u32 = 48;
const BACKGROUND_HEIGHT: u32 = 27;
const BACKGROUND_FILE: &str = "lock-wallpaper.rgb";

/// Matches `crates/rmac-lock-provider-linux/src/paint.rs` `layout::AVATAR_DIAMETER`.
const AVATAR_DIAMETER: u32 = 56;
const AVATAR_FILE: &str = "lock-avatar.rgb";

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".cache"))
        })?;
    Some(base.join("rmac"))
}

/// Reads `name` under the cache directory and builds a [`PictureRaster`]
/// from it, only when the file is exactly `width * height * 3` bytes. A
/// bounded read (one byte over the expected size is enough to reject it)
/// keeps an oversized or unrelated file from costing more than one `read`.
fn load_fixed(name: &str, width: u32, height: u32) -> Option<PictureRaster> {
    let path = cache_dir()?.join(name);
    let expected = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?;
    let expected = expected.checked_mul(3)?;
    let metadata = fs::symlink_metadata(&path).ok()?;
    // No symlinks, no directories/devices/pipes: only a plain file this
    // user process itself is expected to have written.
    if !metadata.file_type().is_file() || metadata.len() != expected as u64 {
        return None;
    }
    let bytes = fs::read(&path).ok()?;
    PictureRaster::new(width, height, bytes)
}

/// The current wallpaper, blurred (LOCK-01). `None` when the resident
/// wallpaper process has not written the cache yet, or wrote something this
/// process will not trust — the caller keeps the Aurora gradient either way.
pub(crate) fn background() -> Option<PictureRaster> {
    load_fixed(BACKGROUND_FILE, BACKGROUND_WIDTH, BACKGROUND_HEIGHT)
}

/// The signed-in user's account picture (LOCK-01). `None` keeps the
/// monogram disc.
pub(crate) fn avatar() -> Option<PictureRaster> {
    load_fixed(AVATAR_FILE, AVATAR_DIAMETER, AVATAR_DIAMETER)
}
