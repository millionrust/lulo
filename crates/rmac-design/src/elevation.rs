//! Shadow elevation. Window shadows are compositor properties expressed as
//! plain data so the niri adapter can consume them.

use rmac_appearance::Contrast;

use crate::color::Rgba;

/// A soft drop shadow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElevationLevel {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur: f32,
    pub spread: f32,
    pub alpha: f32,
}

impl ElevationLevel {
    const NONE: Self = Self {
        offset_x: 0.0,
        offset_y: 0.0,
        blur: 0.0,
        spread: 0.0,
        alpha: 0.0,
    };
}

/// A window shadow as configured on the compositor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowShadow {
    pub softness: f32,
    pub spread: f32,
    pub offset_y: f32,
    pub color: Rgba,
}

/// Every elevation role in the product.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Elevation {
    pub content: ElevationLevel,
    pub raised: ElevationLevel,
    pub popover: ElevationLevel,
    pub modal: ElevationLevel,
    pub window_active: WindowShadow,
    pub window_inactive: WindowShadow,
    pub dock: ElevationLevel,
}

impl Elevation {
    pub fn resolve(contrast: Contrast) -> Self {
        let scale = if contrast == Contrast::Higher {
            1.5
        } else {
            1.0
        };
        let alpha = |value: f32| (value * scale).min(1.0);
        Self {
            content: ElevationLevel::NONE,
            raised: ElevationLevel {
                offset_x: 0.0,
                offset_y: 2.0,
                blur: 8.0,
                spread: 0.0,
                alpha: alpha(0.10),
            },
            popover: ElevationLevel {
                offset_x: 0.0,
                offset_y: 6.0,
                blur: 18.0,
                spread: 0.0,
                alpha: alpha(0.18),
            },
            modal: ElevationLevel {
                offset_x: 0.0,
                offset_y: 10.0,
                blur: 28.0,
                spread: 0.0,
                alpha: alpha(0.24),
            },
            // Measured 2026-09-23 (design-lab/chrome.html): a Gaussian fit
            // to the darkening below TextEdit's bottom edge on a flat
            // backdrop, 0.56 at the edge falling to 0.10 at 40 pt when key,
            // 0.32 falling to 0.08 at 20 pt when not.
            window_active: WindowShadow {
                softness: 42.0,
                spread: 0.0,
                offset_y: 16.0,
                color: Rgba::from_rgba(0x000000bd)
                    .with_alpha((0xbd as f32 * scale).min(255.0) as u8),
            },
            window_inactive: WindowShadow {
                softness: 26.0,
                spread: 0.0,
                offset_y: 8.0,
                color: Rgba::from_rgba(0x00000075)
                    .with_alpha((0x75 as f32 * scale).min(255.0) as u8),
            },
            dock: ElevationLevel {
                offset_x: 0.0,
                offset_y: 8.0,
                blur: 24.0,
                spread: 0.0,
                alpha: alpha(0.20),
            },
        }
    }
}
