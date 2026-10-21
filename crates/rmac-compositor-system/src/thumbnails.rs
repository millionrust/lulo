//! Minimised windows' Dock tiles on Windows: the window's picture, as on
//! Lulo OS (where `minimize_window_in` takes it just before parking).
//!
//! Windows minimises a window itself, from any button or key, and a
//! minimised window draws nothing. So the last picture of each window is
//! kept from the moment it stops being the foreground one (the window
//! a user minimises is usually not the one in front any more by the time
//! Windows reports it, and a background window does not change unseen),
//! and written to the parking store's thumbnail folder when Windows reports
//! the minimise. The Dock shows it as the tile and plays the Genie or Scale
//! effect from it. Pictures are taken on one worker thread, only on those
//! two events; nothing polls. Each kept picture is at most
//! [`LONGEST_SIDE`] pixels, so a dozen windows cost a few megabytes.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

use rmac_compositor as domain;

use crate::picture::{window_picture, Pixels};

/// Longest side of a kept picture, in pixels: a magnified Dock tile at 2×.
const LONGEST_SIDE: u32 = 320;

enum Job {
    /// The window stopped being the foreground one: picture it.
    Keep(isize),
    /// Windows minimised it: write its picture for the Dock, then wake the
    /// watchers.
    Minimized(isize),
}

static WORKER: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();

fn send(job: Job) {
    let worker = WORKER.get_or_init(|| {
        let (sender, jobs) = channel::<Job>();
        let _ = std::thread::Builder::new()
            .name("lulo-window-thumbnails".into())
            .spawn(move || {
                let mut kept: HashMap<isize, Pixels> = HashMap::new();
                for job in jobs {
                    match job {
                        Job::Keep(hwnd) => {
                            if let Some(picture) = window_picture(window(hwnd)).and_then(shrink) {
                                kept.insert(hwnd, picture);
                            }
                        }
                        Job::Minimized(hwnd) => {
                            let picture = kept
                                .remove(&hwnd)
                                .or_else(|| window_picture(window(hwnd)).and_then(shrink));
                            if let Some(picture) = picture {
                                store(window(hwnd), picture);
                            }
                            crate::win32::refresh();
                        }
                    }
                    kept.retain(|hwnd, _| crate::win32::is_window(*hwnd));
                }
            });
        Mutex::new(sender)
    });
    if let Ok(sender) = worker.lock() {
        let _ = sender.send(job);
    }
}

fn window(hwnd: isize) -> domain::WindowId {
    domain::WindowId(hwnd as u64)
}

/// `hwnd` stopped being the foreground window.
pub(crate) fn keep(hwnd: isize) {
    send(Job::Keep(hwnd));
}

/// How long after a window comes to the front it is pictured.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(800);

/// `hwnd` came to the front: picture it once it has drawn, unless it is no
/// longer in front then (it is pictured when it leaves).
pub(crate) fn keep_soon(hwnd: isize) {
    let _ = std::thread::Builder::new()
        .name("lulo-window-thumbnail-wait".into())
        .spawn(move || {
            std::thread::sleep(SETTLE);
            if crate::win32::is_foreground(hwnd) {
                send(Job::Keep(hwnd));
            }
        });
}

/// Windows minimised `hwnd`.
pub(crate) fn minimized(hwnd: isize) {
    send(Job::Minimized(hwnd));
}

/// The picture scaled down to fit [`LONGEST_SIDE`].
pub(crate) fn shrink((width, height, bgra): Pixels) -> Option<Pixels> {
    let image = image::RgbaImage::from_raw(width, height, bgra)?;
    let scale = (LONGEST_SIDE as f32 / width.max(height) as f32).min(1.0);
    if scale >= 1.0 {
        return Some((width, height, image.into_raw()));
    }
    let (small_width, small_height) = (
        ((width as f32 * scale).round() as u32).max(1),
        ((height as f32 * scale).round() as u32).max(1),
    );
    let small = image::imageops::resize(
        &image,
        small_width,
        small_height,
        image::imageops::FilterType::Triangle,
    );
    Some((small_width, small_height, small.into_raw()))
}

/// Write the picture as the window's thumbnail (PNG, RGBA), replacing any
/// earlier one.
fn store(window: domain::WindowId, (width, height, bgra): Pixels) {
    let Some(dir) = domain::ParkingStore::default_thumbnail_dir() else {
        return;
    };
    domain::ParkingStore::remove_thumbnails_in(&dir, window);
    let mut rgba = bgra;
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let Some(image) = image::RgbaImage::from_raw(width, height, rgba) else {
        return;
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default();
    let path = domain::ParkingStore::thumbnail_path_in(&dir, window, stamp);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = image.save_with_format(&path, image::ImageFormat::Png) {
        eprintln!("no minimised-window thumbnail: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_pictures_fit_a_dock_tile_and_keep_their_shape() {
        let (width, height, _) = shrink((1280, 800, vec![0; 1280 * 800 * 4])).unwrap();
        assert_eq!((width, height), (320, 200));
        let small = shrink((100, 50, vec![0; 100 * 50 * 4])).unwrap();
        assert_eq!((small.0, small.1), (100, 50));
        assert!(shrink((10, 10, vec![0; 3])).is_none());
    }
}
