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
            window_active: WindowShadow {
                softness: 32.0,
                spread: 0.0,
                offset_y: 12.0,
                color: Rgba::from_rgba(0x00000073)
                    .with_alpha((0x73 as f32 * scale).min(255.0) as u8),
            },
            window_inactive: WindowShadow {
                softness: 12.0,
                spread: 0.0,
                offset_y: 4.0,
                color: Rgba::from_rgba(0x0000004d)
                    .with_alpha((0x4d as f32 * scale).min(255.0) as u8),
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
