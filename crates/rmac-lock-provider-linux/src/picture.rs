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
use std::io::Read as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

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
/// from it, only when the file is exactly `width * height * 3` bytes.
fn load_fixed(name: &str, width: u32, height: u32) -> Option<PictureRaster> {
    let expected = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?;
    let expected = expected.checked_mul(3)?;
    let bytes = read_exact_plain_file(&cache_dir()?.join(name), expected)?;
    PictureRaster::new(width, height, bytes)
}

/// The contents of `path` when it is a plain file of exactly `expected`
/// bytes, else `None`. The checks apply to the descriptor actually read, not
/// to the path before opening it (SR-28): a link is refused by `O_NOFOLLOW`,
/// a FIFO or device swapped in cannot block the lock screen (`O_NONBLOCK`,
/// then `fstat` must say regular file), and the read stops one byte past
/// `expected`, so a file that grows after the check costs no more than that.
fn read_exact_plain_file(path: &Path, expected: usize) -> Option<Vec<u8>> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.file_type().is_file() || metadata.len() != expected as u64 {
        return None;
    }
    let mut bytes = Vec::with_capacity(expected);
    file.take(expected as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() == expected).then_some(bytes)
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

#[cfg(test)]
mod tests {
    use super::read_exact_plain_file;
    use std::path::PathBuf;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("rmac-lock-picture-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_plain_file_of_the_exact_size_is_read() {
        let scratch = Scratch::new("exact");
        let path = scratch.0.join("picture.rgb");
        std::fs::write(&path, [7u8; 12]).unwrap();
        assert_eq!(read_exact_plain_file(&path, 12), Some(vec![7u8; 12]));
    }

    #[test]
    fn a_file_of_any_other_size_is_refused() {
        let scratch = Scratch::new("size");
        let path = scratch.0.join("picture.rgb");
        std::fs::write(&path, [7u8; 13]).unwrap();
        assert_eq!(read_exact_plain_file(&path, 12), None);
        std::fs::write(&path, [7u8; 11]).unwrap();
        assert_eq!(read_exact_plain_file(&path, 12), None);
    }

    #[test]
    fn a_link_to_a_plain_file_of_the_right_size_is_refused() {
        let scratch = Scratch::new("link");
        let target = scratch.0.join("elsewhere.rgb");
        std::fs::write(&target, [7u8; 12]).unwrap();
        let link = scratch.0.join("picture.rgb");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(read_exact_plain_file(&link, 12), None);
    }

    #[test]
    fn a_fifo_is_refused_without_blocking_the_lock_screen() {
        let scratch = Scratch::new("fifo");
        let fifo = scratch.0.join("picture.rgb");
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: a valid NUL-terminated path; mkfifo has no other
        // preconditions.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        // With no writer, a blocking open would hang here forever.
        assert_eq!(read_exact_plain_file(&fifo, 12), None);
    }

    #[test]
    fn a_directory_is_refused() {
        let scratch = Scratch::new("dir");
        let directory = scratch.0.join("picture.rgb");
        std::fs::create_dir(&directory).unwrap();
        assert_eq!(read_exact_plain_file(&directory, 0), None);
    }
}
