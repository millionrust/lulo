//! Minimize any window the way the yellow traffic light does (§2.2, §4.11).
//!
//! niri has no minimized state, so rmac parks a window on the hidden
//! `rmac-parking` workspace. Every entry point shares this one path: the
//! yellow button and ⌘M inside rmac apps, the ⌘M binding for every other
//! application (`mission-control minimize`), and a third-party window's own
//! minimize button, which Lulo's niri reports as `WindowMinimizeRequested`
//! (docs/decisions/0021-niri-minimize-request.md). In order it
//!
//! 1. records the workspace the window came from, so the Dock can restore it;
//! 2. pictures the window while it is still on screen, for the Dock's tile;
//! 3. parks it.

use std::process::{Command, Stdio};

use super::*;

/// The niri event a patched niri sends when a window asks to be minimized.
pub const MINIMIZE_REQUEST_EVENT: &str = "WindowMinimizeRequested";

/// Longest side of a stored thumbnail, in pixels: a magnified Dock tile at
/// scale 2 is well under this, and a small file keeps the Dock light.
const THUMBNAIL_MAX_SIDE: u32 = 320;

/// The window a `WindowMinimizeRequested` event names, if `event` is one.
pub fn minimize_request(event: &domain::Event) -> Option<domain::WindowId> {
    let domain::Event::Unknown {
        source_kind,
        payload,
    } = event
    else {
        return None;
    };
    if source_kind != MINIMIZE_REQUEST_EVENT {
        return None;
    }
    payload
        .get("id")
        .and_then(Value::as_u64)
        .map(domain::WindowId)
}

/// Why a window could not be minimized.
#[derive(Debug)]
pub enum MinimizeError {
    Snapshot(Error),
    Worker(io::Error),
    Action(domain::ActionError),
}

impl fmt::Display for MinimizeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Snapshot(error) => write!(formatter, "could not read the windows: {error}"),
            Self::Worker(error) => {
                write!(formatter, "could not start the minimize worker: {error}")
            }
            Self::Action(error) => write!(formatter, "could not park the window: {error:?}"),
        }
    }
}

impl std::error::Error for MinimizeError {}

/// Minimize whichever window niri reports focused. Returns the window, or
/// `None` when nothing is focused.
pub async fn minimize_focused_window() -> Result<Option<domain::WindowId>, MinimizeError> {
    let snapshot = snapshot().await.map_err(MinimizeError::Snapshot)?;
    let Some(window) = snapshot.focus.window else {
        return Ok(None);
    };
    minimize_window_in(snapshot, window).await?;
    Ok(Some(window))
}

/// Record, picture and park `window`. A window that is already parked, or
/// no longer exists, is left alone.
pub async fn minimize_window(window: domain::WindowId) -> Result<(), MinimizeError> {
    let snapshot = snapshot().await.map_err(MinimizeError::Snapshot)?;
    minimize_window_in(snapshot, window).await
}

/// [`minimize_window`] with a snapshot the caller already read.
pub async fn minimize_window_in(
    snapshot: domain::Snapshot,
    window: domain::WindowId,
) -> Result<(), MinimizeError> {
    let Some(candidate) = snapshot.windows.iter().find(|w| w.id == window) else {
        return Ok(());
    };
    if domain::window_is_parked(&snapshot, candidate) {
        return Ok(());
    }
    // The store and grim are blocking; keep them off the caller's executor.
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name("rmac-minimize".into())
        .spawn(move || {
            record_and_capture(&snapshot, window);
            let _ = sender.send_blocking(());
        })
        .map_err(MinimizeError::Worker)?;
    let _ = receiver.recv().await;
    execute_action(&domain::Action::MinimizeWindow { window })
        .await
        .map_err(MinimizeError::Action)
}

/// Steps 1 and 2: remember where `window` came from and picture it. Both
/// are best effort; parking goes ahead without them (the Dock then shows
/// the application icon, and restores to the focused Space).
pub fn record_and_capture(snapshot: &domain::Snapshot, window: domain::WindowId) {
    let mut store = domain::ParkingStore::load_default();
    for dropped in store.prune(snapshot) {
        if let Some(dir) = domain::ParkingStore::default_thumbnail_dir() {
            domain::ParkingStore::remove_thumbnails_in(&dir, dropped.window);
        }
    }
    store.record_from(snapshot, &[window]);
    if let Some(dir) = domain::ParkingStore::default_thumbnail_dir() {
        // An earlier minimize's picture of the same window is stale now.
        domain::ParkingStore::remove_thumbnails_in(&dir, window);
        if let Some(rect) = capture_rect(snapshot, window) {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis())
                .unwrap_or_default();
            let path = domain::ParkingStore::thumbnail_path_in(&dir, window, stamp);
            match capture_thumbnail(rect, &path) {
                Ok(()) => {
                    store.set_thumbnail(window, path);
                }
                Err(reason) => eprintln!("no minimized-window thumbnail: {reason}"),
            }
        }
    }
    if let Err(error) = store.save_default() {
        eprintln!("could not save the parking set: {error}");
    }
}

/// Where to picture `window`, or `None` when it is not actually on screen:
/// its Space is not the one showing, or the overview is covering it. grim
/// copies the screen, so capturing then would picture something else.
pub fn capture_rect(
    snapshot: &domain::Snapshot,
    window: domain::WindowId,
) -> Option<domain::LogicalRect> {
    if snapshot.overview_visible {
        return None;
    }
    let workspace = snapshot
        .windows
        .iter()
        .find(|candidate| candidate.id == window)?
        .workspace?;
    let showing = snapshot
        .workspaces
        .iter()
        .any(|candidate| candidate.id == workspace && candidate.active);
    if !showing {
        return None;
    }
    domain::window_logical_rect(snapshot, window)
        .filter(|rect| rect.width >= 1.0 && rect.height >= 1.0)
}

/// Copy `rect` off the screen with grim and store it as a PNG at `path`.
/// A blank picture (a powered-off or locked screen, or a window that blocks
/// capture) is refused, so the Dock falls back to the application icon
/// rather than showing a black tile.
fn capture_thumbnail(rect: domain::LogicalRect, path: &Path) -> Result<(), String> {
    let geometry = format!(
        "{:.0},{:.0} {:.0}x{:.0}",
        rect.x.round(),
        rect.y.round(),
        rect.width.round(),
        rect.height.round()
    );
    let output = Command::new("grim")
        .args(["-t", "ppm", "-g", &geometry, "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| format!("could not run grim: {error}"))?;
    if !output.status.success() {
        return Err(format!("grim failed: {}", output.status));
    }
    let picture = image::load_from_memory_with_format(&output.stdout, image::ImageFormat::Pnm)
        .map_err(|error| format!("grim wrote an unreadable picture: {error}"))?
        .into_rgb8();
    if is_blank(&picture) {
        return Err(format!("the screen at {geometry} is blank"));
    }
    let picture = shrink(picture, THUMBNAIL_MAX_SIDE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {parent:?}: {error}"))?;
    }
    // Write beside the target and rename, so the Dock never reads half a file.
    let partial = path.with_extension("png.part");
    picture
        .save_with_format(&partial, image::ImageFormat::Png)
        .map_err(|error| format!("could not write {partial:?}: {error}"))?;
    std::fs::rename(&partial, path).map_err(|error| format!("could not move {path:?}: {error}"))
}

/// True when every pixel is within a few levels of the first one: nothing
/// but a flat colour, which a real window never is.
pub fn is_blank(picture: &image::RgbImage) -> bool {
    const TOLERANCE: u8 = 6;
    let mut pixels = picture.pixels();
    let Some(first) = pixels.next() else {
        return true;
    };
    pixels.all(|pixel| {
        pixel
            .0
            .iter()
            .zip(first.0.iter())
            .all(|(channel, reference)| channel.abs_diff(*reference) <= TOLERANCE)
    })
}

/// Scale `picture` down so its longer side is at most `max_side`.
pub fn shrink(picture: image::RgbImage, max_side: u32) -> image::RgbImage {
    let (width, height) = picture.dimensions();
    let longest = width.max(height);
    if longest <= max_side {
        return picture;
    }
    let scale = f64::from(max_side) / f64::from(longest);
    let target_width = ((f64::from(width) * scale).round() as u32).max(1);
    let target_height = ((f64::from(height) * scale).round() as u32).max(1);
    image::imageops::resize(
        &picture,
        target_width,
        target_height,
        image::imageops::FilterType::Triangle,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimize_request_event_names_the_window() {
        // Lulo's niri patch (docs/decisions/0021-niri-minimize-request.md)
        // writes exactly this line; the adapter keeps it as an Unknown event.
        let decoded = decode_event(r#"{"WindowMinimizeRequested":{"id":7}}"#).unwrap();
        let events = translate(
            decoded.event,
            decoded.source_kind,
            decoded.payload,
            &domain::State::default(),
        );
        assert_eq!(minimize_request(&events[0]), Some(domain::WindowId(7)));

        let other = domain::Event::Unknown {
            source_kind: "ScreenshotCaptured".into(),
            payload: serde_json::json!({ "id": 7 }),
        };
        assert_eq!(minimize_request(&other), None);
        let malformed = domain::Event::Unknown {
            source_kind: MINIMIZE_REQUEST_EVENT.into(),
            payload: serde_json::json!({ "id": "seven" }),
        };
        assert_eq!(minimize_request(&malformed), None);
        assert_eq!(
            minimize_request(&domain::Event::WindowRemoved {
                id: domain::WindowId(7)
            }),
            None
        );
    }

    fn snapshot() -> domain::Snapshot {
        let workspace = |id: u64, active: bool, name: Option<&str>| domain::Workspace {
            id: domain::WorkspaceId(id),
            index: id as u8,
            name: name.map(str::to_owned),
            output: Some(domain::OutputId::from("eDP-1")),
            urgent: false,
            active,
            focused: active,
            active_window: None,
        };
        let window = |id: u64, workspace: u64| domain::Window {
            id: domain::WindowId(id),
            title: None,
            app_id: Some("firefox".into()),
            pid: Some(42),
            workspace: Some(domain::WorkspaceId(workspace)),
            focused: false,
            floating: true,
            urgent: false,
            focus_timestamp: None,
            layout: domain::WindowLayout {
                tile_position_in_view: Some(domain::LogicalPoint { x: 100.0, y: 50.0 }),
                tile_size: domain::LogicalSize {
                    width: 400.0,
                    height: 300.0,
                },
                ..Default::default()
            },
        };
        domain::Snapshot {
            outputs: vec![domain::Output {
                id: domain::OutputId::from("eDP-1"),
                make: String::new(),
                model: String::new(),
                serial: None,
                physical_size_mm: None,
                modes: vec![],
                current_mode: Some(0),
                custom_mode: false,
                vrr_supported: false,
                vrr_enabled: false,
                logical: Some(domain::LogicalOutput {
                    position: domain::LogicalPoint { x: 0.0, y: 0.0 },
                    size: domain::LogicalSize {
                        width: 1536.0,
                        height: 864.0,
                    },
                    scale: 1.25,
                    transform: "normal".into(),
                }),
            }],
            workspaces: vec![
                workspace(1, true, None),
                workspace(2, false, None),
                workspace(9, false, Some(domain::PARKING_WORKSPACE)),
            ],
            windows: vec![window(1, 1), window(2, 2)],
            ..Default::default()
        }
    }

    #[test]
    fn thumbnails_are_taken_only_of_windows_on_screen() {
        let mut snapshot = snapshot();
        assert_eq!(
            capture_rect(&snapshot, domain::WindowId(1)),
            Some(domain::LogicalRect {
                x: 100.0,
                y: 50.0,
                width: 400.0,
                height: 300.0,
            })
        );
        // On a Space that is not showing, grim would picture something else.
        assert_eq!(capture_rect(&snapshot, domain::WindowId(2)), None);
        assert_eq!(capture_rect(&snapshot, domain::WindowId(99)), None);
        // The overview covers every window.
        snapshot.overview_visible = true;
        assert_eq!(capture_rect(&snapshot, domain::WindowId(1)), None);
    }

    #[test]
    fn blank_pictures_are_refused_and_real_ones_kept() {
        let black = image::RgbImage::from_pixel(40, 30, image::Rgb([0, 0, 0]));
        assert!(is_blank(&black));
        // Dithering noise on a blanked screen is still blank.
        let mut noisy = black.clone();
        noisy.put_pixel(3, 4, image::Rgb([4, 2, 5]));
        assert!(is_blank(&noisy));
        // A dark window with one lit control is a real picture.
        let mut dark_window = black;
        dark_window.put_pixel(10, 10, image::Rgb([255, 149, 0]));
        assert!(!is_blank(&dark_window));
        assert!(is_blank(&image::RgbImage::new(0, 0)));
    }

    #[test]
    fn thumbnails_shrink_to_the_longest_side_and_keep_their_shape() {
        let small = shrink(image::RgbImage::new(287, 507), 320);
        assert_eq!(small.dimensions(), (181, 320));
        assert_eq!(
            shrink(image::RgbImage::new(100, 50), 320).dimensions(),
            (100, 50)
        );
    }
}
