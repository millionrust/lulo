//! The volume and brightness OSD as macOS 26 draws it: the Control Center
//! Sound or Display module (design-lab/switcher-osd.html, measured on the
//! owner's Mac), 292 × 64, radius 22, 14 from the right edge, 6 below the
//! bar. One view for Lulo OS's `osd` service and the Windows Lulo layer.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{div, img, prelude::*, px, relative, rgba, Div, ElementId, FontWeight, Role};

use crate::{Kind, Presentation};

pub const SURFACE_WIDTH: f32 = 292.0;
pub const SURFACE_HEIGHT: f32 = 64.0;
pub const RADIUS: f32 = 22.0;
pub const RIGHT_MARGIN: f32 = 14.0;
pub const GAP_BELOW_BAR: f32 = 6.0;
pub const HIDE_DELAY: Duration = Duration::from_millis(1_600);
/// Title: 13 pt bold at x 16.5, baseline 23 (a 16 pt line box from 10.5).
const TITLE_LEFT: f32 = 16.5;
const TITLE_TOP: f32 = 10.5;
/// Slider: 4 thick on the centre line y = 41.75, no knob.
const ROW_CENTER: f32 = 41.75;
const ROW_HEIGHT: f32 = 17.0;
const TRACK_HEIGHT: f32 = 4.0;
/// The OSD has no trailing AirPlay/display button, so the row mirrors the
/// module's left inset on the right.
const ROW_RIGHT: f32 = 16.0;
/// Dark glass measured over near-black content: #2E3034 → 52,54,58 @ 85 %.
const DARK_TINT: u32 = 0x3436_3AD9;
const DARK_RIM: u32 = 0xFFFF_FF66;
const DARK_TRACK: u32 = 0x0000_0066;
const DARK_FILL: u32 = 0xFAFA_FAFF;
const LIGHT_TINT: u32 = 0xF6F6_F8D9;
const LIGHT_RIM: u32 = 0xFFFF_FFB3;
const LIGHT_TRACK: u32 = 0x0000_001A;
const LIGHT_FILL: u32 = 0x1D1D_1FFF;

/// One glyph with its measured size in points (the SVGs are cropped to the
/// glyph, so the box is the drawn size).
struct Glyph {
    file: &'static str,
    width: f32,
    height: f32,
}

/// Leading glyph, gap to the track, trailing gap, trailing glyph.
struct RowGlyphs {
    low: Glyph,
    low_left: f32,
    low_gap: f32,
    high_gap: f32,
    high: Glyph,
}

/// The glyph files the OSD draws (`shell/assets/osd`), for installers.
pub const GLYPH_FILES: [&str; 7] = [
    "speaker-low.svg",
    "speaker-high.svg",
    "muted.svg",
    "microphone-low.svg",
    "microphone-high.svg",
    "brightness-low.svg",
    "brightness-high.svg",
];

/// Control Center's module names: the OSD reads "Sound" or "Display" on the
/// Mac; the device name stays in the accessible label.
pub fn title(presentation: &Presentation) -> String {
    match presentation.kind {
        Kind::Output => "Sound".to_owned(),
        Kind::Display => "Display".to_owned(),
        Kind::Keyboard => "Keyboard Brightness".to_owned(),
        Kind::Input => "Microphone".to_owned(),
    }
}

fn row_glyphs(presentation: &Presentation) -> RowGlyphs {
    // Measured (pt): speaker.fill 9.25 × 13.5 at x 16.5, track from 34,
    // speaker.wave.3.fill 20.5 × 14.75 seven after the track; sun.min.fill
    // 15 at x 15.5, track from 36.5, sun.max.fill 16.5 six after it.
    const SPEAKER: Glyph = Glyph {
        file: "speaker-low.svg",
        width: 9.25,
        height: 13.5,
    };
    const SPEAKER_WAVES: Glyph = Glyph {
        file: "speaker-high.svg",
        width: 20.5,
        height: 14.75,
    };
    const MUTED: Glyph = Glyph {
        file: "muted.svg",
        width: 11.0,
        height: 13.5,
    };
    match (presentation.kind, presentation.muted) {
        (Kind::Output, muted) => RowGlyphs {
            low: if muted { MUTED } else { SPEAKER },
            low_left: 16.5,
            low_gap: if muted { 6.5 } else { 8.25 },
            high_gap: 7.0,
            high: SPEAKER_WAVES,
        },
        (Kind::Input, muted) => RowGlyphs {
            low: if muted {
                MUTED
            } else {
                Glyph {
                    file: "microphone-low.svg",
                    width: 15.0,
                    height: 15.0,
                }
            },
            low_left: 15.5,
            low_gap: 6.0,
            high_gap: 6.0,
            high: Glyph {
                file: "microphone-high.svg",
                width: 17.0,
                height: 17.0,
            },
        },
        (Kind::Display | Kind::Keyboard, _) => RowGlyphs {
            low: Glyph {
                file: "brightness-low.svg",
                width: 15.0,
                height: 15.0,
            },
            low_left: 15.5,
            low_gap: 6.0,
            high_gap: 6.0,
            high: Glyph {
                file: "brightness-high.svg",
                width: 16.5,
                height: 16.5,
            },
        },
    }
}

/// The OSD panel for `presentation`, filling its surface; `glyph` turns a
/// glyph file name into the path it is installed at.
pub fn panel(
    presentation: &Presentation,
    id: impl Into<ElementId>,
    dark: bool,
    glyph: impl Fn(&str) -> PathBuf,
) -> impl IntoElement {
    let glyphs = row_glyphs(presentation);
    let fraction = f32::from(presentation.visible_level()) / 100.0;
    let label = presentation.accessible_label();
    let (tint, rim, track, fill) = if dark {
        (DARK_TINT, DARK_RIM, DARK_TRACK, DARK_FILL)
    } else {
        (LIGHT_TINT, LIGHT_RIM, LIGHT_TRACK, LIGHT_FILL)
    };
    let text = if dark { 0xFFFF_FFFF } else { 0x0000_00E6 };
    let row: Div = div()
        .absolute()
        .left_0()
        .right(px(ROW_RIGHT))
        .top(px(ROW_CENTER - ROW_HEIGHT / 2.0))
        .h(px(ROW_HEIGHT))
        .flex()
        .items_center()
        .child(
            img(glyph(glyphs.low.file))
                .flex_none()
                .ml(px(glyphs.low_left))
                .mr(px(glyphs.low_gap))
                .w(px(glyphs.low.width))
                .h(px(glyphs.low.height)),
        )
        .child(
            div()
                .relative()
                .flex_1()
                .h(px(TRACK_HEIGHT))
                .rounded_full()
                .bg(rgba(track))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .h_full()
                        .w(relative(fraction))
                        .rounded_full()
                        .bg(rgba(fill)),
                ),
        )
        .child(
            img(glyph(glyphs.high.file))
                .flex_none()
                .ml(px(glyphs.high_gap))
                .w(px(glyphs.high.width))
                .h(px(glyphs.high.height)),
        );
    div()
        .id(id)
        .relative()
        .size_full()
        .rounded(px(RADIUS))
        .bg(rgba(tint))
        .border(px(0.5))
        .border_color(rgba(rim))
        .role(Role::Status)
        .aria_label(label)
        .child(
            div()
                .absolute()
                .left(px(TITLE_LEFT))
                .right(px(ROW_RIGHT))
                .top(px(TITLE_TOP))
                .h(px(16.0))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_size(px(13.0))
                .line_height(px(16.0))
                .font_weight(FontWeight::BOLD)
                .text_color(rgba(text))
                .child(title(presentation)),
        )
        .child(row)
}
