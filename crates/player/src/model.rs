//! Player arithmetic shared by the window and the MPRIS service: time text,
//! scrubbing, volume, the controller's auto-hide and the window size.

/// "00:07" / "1:02:03"; the Mac's elapsed and duration labels.
pub fn clock_text(seconds: f64) -> String {
    let total = if seconds.is_finite() && seconds > 0.0 {
        seconds.floor() as u64
    } else {
        0
    };
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// "-00:13": time remaining, shown when the duration label is toggled.
pub fn remaining_text(position: f64, duration: f64) -> String {
    format!("-{}", clock_text((duration - position).max(0.0).ceil()))
}

/// Position for a pointer at `x` along a track `width` wide.
pub fn scrub_position(x: f32, width: f32, duration: f64) -> f64 {
    if width <= 0.0 || !duration.is_finite() || duration <= 0.0 {
        return 0.0;
    }
    f64::from((x / width).clamp(0.0, 1.0)) * duration
}

/// Fraction of the track that has played.
pub fn progress(position: f64, duration: f64) -> f32 {
    if duration.is_finite() && duration > 0.0 && position.is_finite() {
        (position / duration).clamp(0.0, 1.0) as f32
    } else {
        0.0
    }
}

/// mpv volume (0–100) ⇄ MPRIS volume (0.0–1.0).
pub fn mpris_volume(mpv: f64) -> f64 {
    (mpv / 100.0).clamp(0.0, 1.0)
}

pub fn mpv_volume(mpris: f64) -> f64 {
    if mpris.is_finite() {
        (mpris * 100.0).clamp(0.0, 100.0)
    } else {
        100.0
    }
}

/// MPRIS positions are microseconds.
pub fn microseconds(seconds: f64) -> i64 {
    if seconds.is_finite() {
        (seconds * 1_000_000.0).round() as i64
    } else {
        0
    }
}

/// A relative MPRIS Seek clamped to the track; `None` means "past the end",
/// which the spec treats as Next.
pub fn seek_target(position: f64, offset_microseconds: i64, duration: f64) -> Option<f64> {
    let target = position + offset_microseconds as f64 / 1_000_000.0;
    if duration.is_finite() && duration > 0.0 && target >= duration {
        return None;
    }
    Some(target.max(0.0))
}

/// When the floating controller and title bar hide: 3 s after the pointer
/// last moved while a video plays (S: QuickTime's delay by eye).
pub const CONTROLS_HIDE_AFTER_MS: u64 = 3_000;

pub fn controls_visible(
    playing: bool,
    is_video: bool,
    idle_ms: u64,
    pointer_inside_controls: bool,
) -> bool {
    !is_video || !playing || pointer_inside_controls || idle_ms < CONTROLS_HIDE_AFTER_MS
}

/// Window content size for a video: its own size, scaled down (never up)
/// to fit `limit`, and never smaller than the controller needs.
pub fn video_window_size(video: (f64, f64), limit: (f32, f32), minimum: (f32, f32)) -> (f32, f32) {
    let (width, height) = video;
    if !(width > 0.0 && height > 0.0) {
        return (minimum.0.max(640.0), minimum.1.max(360.0));
    }
    let scale = (f64::from(limit.0) / width)
        .min(f64::from(limit.1) / height)
        .min(1.0);
    let fitted = ((width * scale) as f32, (height * scale) as f32);
    (fitted.0.max(minimum.0), fitted.1.max(minimum.1))
}

/// Largest render target (device pixels) for the software video path; the
/// picture is scaled up by the GPU beyond this. Keeps a low-end CPU from
/// converting 4K frames it cannot show.
pub const MAX_RENDER_PIXELS: (u32, u32) = (1920, 1080);

/// Pixel size to ask libmpv for: the displayed box at the window's scale,
/// capped, with the width a multiple of 16 so rows stay 64-byte aligned.
pub fn render_size(display: (f32, f32), scale: f32, video: (f64, f64)) -> Option<(u32, u32)> {
    let (box_width, box_height) = (display.0 * scale, display.1 * scale);
    let (video_width, video_height) = video;
    if !(box_width >= 16.0 && box_height >= 16.0 && video_width > 0.0 && video_height > 0.0) {
        return None;
    }
    // Contain the video in the box.
    let fit = (f64::from(box_width) / video_width).min(f64::from(box_height) / video_height);
    let mut width = (video_width * fit).min(f64::from(MAX_RENDER_PIXELS.0));
    let mut height = (video_height * fit).min(f64::from(MAX_RENDER_PIXELS.1));
    let aspect = video_width / video_height;
    if width / height > aspect {
        width = height * aspect;
    } else {
        height = width / aspect;
    }
    let width = ((width as u32) / 16 * 16).max(16);
    let height = (height as u32).max(16);
    Some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_labels() {
        assert_eq!(clock_text(0.0), "00:00");
        assert_eq!(clock_text(7.9), "00:07");
        assert_eq!(clock_text(20.0), "00:20");
        assert_eq!(clock_text(3723.0), "1:02:03");
        assert_eq!(clock_text(f64::NAN), "00:00");
        assert_eq!(remaining_text(7.2, 20.0), "-00:13");
        assert_eq!(remaining_text(25.0, 20.0), "-00:00");
    }

    #[test]
    fn scrubbing_and_progress() {
        assert_eq!(scrub_position(163.0, 326.0, 20.0), 10.0);
        assert_eq!(scrub_position(-5.0, 326.0, 20.0), 0.0);
        assert_eq!(scrub_position(999.0, 326.0, 20.0), 20.0);
        assert_eq!(scrub_position(10.0, 0.0, 20.0), 0.0);
        assert_eq!(progress(5.0, 20.0), 0.25);
        assert_eq!(progress(5.0, f64::NAN), 0.0);
    }

    #[test]
    fn volume_and_mpris_units() {
        assert_eq!(mpris_volume(80.0), 0.8);
        assert_eq!(mpris_volume(130.0), 1.0);
        assert_eq!(mpv_volume(0.5), 50.0);
        assert_eq!(mpv_volume(-1.0), 0.0);
        assert_eq!(microseconds(1.5), 1_500_000);
        assert_eq!(seek_target(10.0, 5_000_000, 20.0), Some(15.0));
        assert_eq!(seek_target(10.0, -30_000_000, 20.0), Some(0.0));
        assert_eq!(seek_target(10.0, 10_000_000, 20.0), None);
    }

    #[test]
    fn controls_hide_only_for_playing_video() {
        assert!(controls_visible(true, true, 1_000, false));
        assert!(!controls_visible(true, true, 3_000, false));
        assert!(controls_visible(true, true, 9_000, true));
        assert!(controls_visible(false, true, 9_000, false));
        assert!(controls_visible(true, false, 9_000, false));
    }

    #[test]
    fn window_fits_the_video() {
        assert_eq!(
            video_window_size((1280.0, 720.0), (1600.0, 1000.0), (480.0, 270.0)),
            (1280.0, 720.0)
        );
        assert_eq!(
            video_window_size((3840.0, 2160.0), (1600.0, 1000.0), (480.0, 270.0)),
            (1600.0, 900.0)
        );
        assert_eq!(
            video_window_size((320.0, 240.0), (1600.0, 1000.0), (480.0, 270.0)),
            (480.0, 270.0)
        );
        assert_eq!(
            video_window_size((0.0, 0.0), (1600.0, 1000.0), (480.0, 270.0)),
            (640.0, 360.0)
        );
    }

    #[test]
    fn render_targets_are_capped_and_aligned() {
        // A 1280 × 720 window at 2× shows 2560 × 1440 → capped to 1920 × 1080.
        assert_eq!(
            render_size((1280.0, 720.0), 2.0, (1920.0, 1080.0)),
            Some((1920, 1080))
        );
        // Letterboxed 4:3 in a wide box.
        let (width, height) = render_size((1000.0, 400.0), 1.0, (640.0, 480.0)).unwrap();
        assert_eq!(height, 400);
        assert_eq!(width % 16, 0);
        assert!((width as i32 - 533).abs() <= 16);
        assert_eq!(render_size((10.0, 10.0), 1.0, (640.0, 480.0)), None);
        assert_eq!(render_size((500.0, 500.0), 1.0, (0.0, 0.0)), None);
    }
}
