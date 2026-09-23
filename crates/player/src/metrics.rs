//! QuickTime Player's measured geometry (dark appearance), in points from
//! the window's top-left. Source: AX frames of a 1280 × 720 video window and
//! an audio window, 2026-09-23; see design-lab/apps-clock-weather-player.html.
//! `S` marks values drawn from knowledge.

/// Video: the 32 pt title bar overlays the picture.
pub const TITLE_BAR_HEIGHT: f32 = 32.0;
pub const TRAFFIC_LIGHT_CENTER: (f32, f32) = (16.0, 16.0);
/// Title: document icon 18 pt at x 81, name 13 bold at x 100.
pub const TITLE_ICON_LEFT: f32 = 81.0;
pub const TITLE_LEFT: f32 = 100.0;
pub const TITLE_SIZE: f32 = 13.0;

/// Floating controller (S: plate size; control frames are AX-measured).
pub const HUD_WIDTH: f32 = 459.0;
pub const HUD_HEIGHT: f32 = 92.0;
/// Plate bottom 124 above the window's bottom (504 + 92 of 720).
pub const HUD_BOTTOM: f32 = 124.0;
pub const HUD_RADIUS: f32 = 18.0;
/// rgba.
pub const HUD_FILL: u32 = 0x1E1E_1ED1;
pub const HUD_RIM: u32 = 0xFFFF_FF2E;
/// Inside the plate: row 1 at y 16–36, row 2 at y 60–77.
pub const HUD_MUTE: (f32, f32) = (16.0, 20.0);
pub const HUD_VOLUME: (f32, f32, f32) = (40.0, 26.0, 83.0);
pub const HUD_REWIND_CENTER: (f32, f32) = (190.5, 25.5);
pub const HUD_PLAY_CENTER: (f32, f32) = (229.5, 26.0);
pub const HUD_FORWARD_CENTER: (f32, f32) = (269.5, 25.5);
pub const HUD_FULLSCREEN_CENTER: (f32, f32) = (428.0, 25.5);
pub const HUD_ELAPSED: (f32, f32) = (16.0, 60.0);
pub const HUD_TIMELINE: (f32, f32, f32) = (67.0, 68.5, 326.0);
pub const HUD_DURATION_RIGHT: f32 = 16.0;

/// Audio-only window: 350 × 135.
pub const AUDIO_WINDOW: (f32, f32) = (350.0, 135.0);
pub const AUDIO_ELAPSED: (f32, f32) = (11.0, 39.0);
pub const AUDIO_TIMELINE: (f32, f32, f32) = (61.0, 47.5, 227.0);
pub const AUDIO_DURATION: (f32, f32) = (295.0, 39.0);
pub const AUDIO_REWIND_CENTER: (f32, f32) = (125.5, 79.5);
pub const AUDIO_PLAY_CENTER: (f32, f32) = (174.5, 79.0);
pub const AUDIO_FORWARD_CENTER: (f32, f32) = (224.5, 79.5);
pub const AUDIO_MUTE: (f32, f32) = (11.0, 105.0);
pub const AUDIO_VOLUME: (f32, f32, f32) = (35.0, 111.0, 302.0);
/// Measured through the translucent plate over a coloured video (S: the
/// Mac blurs what is behind it).
pub const AUDIO_FILL: u32 = 0x2624_24F0;

/// Tracks: 6 pt tall, 20 % white, played part white.
pub const TRACK_HEIGHT: f32 = 6.0;
pub const TRACK_FILL: u32 = 0xFFFF_FF33;
pub const THUMB_DIAMETER: f32 = 16.0;
pub const VOLUME_THUMB_DIAMETER: f32 = 20.0;
pub const TIME_SIZE: f32 = 13.0;
pub const TIME_WIDTH: f32 = 43.0;
pub const TRANSPORT_GLYPH: f32 = 22.0;
pub const PLAY_GLYPH: f32 = 26.0;
pub const SPEAKER_GLYPH: f32 = 17.0;

/// Video windows fit within this before the first frame (S).
pub const DEFAULT_VIDEO_WINDOW: (f32, f32) = (1280.0, 720.0);
pub const MIN_VIDEO_WINDOW: (f32, f32) = (480.0, 270.0);
