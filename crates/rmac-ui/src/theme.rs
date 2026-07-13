//! Semantic design tokens derived from one resolved appearance snapshot.

use gpui::{rgb, rgba, FontWeight, Hsla};
use rmac_appearance::{
    AccentColor, Contrast, MotionPreference, ResolvedAppearance, ResolvedColorScheme, TextScale,
};
use std::sync::{OnceLock, RwLock};

static CURRENT_TOKENS: OnceLock<RwLock<ThemeTokens>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RgbaColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl RgbaColor {
    pub const fn opaque(hex: u32) -> Self {
        Self {
            red: ((hex >> 16) & 0xff) as u8,
            green: ((hex >> 8) & 0xff) as u8,
            blue: (hex & 0xff) as u8,
            alpha: 0xff,
        }
    }

    pub const fn with_alpha(hex: u32, alpha: u8) -> Self {
        Self {
            alpha,
            ..Self::opaque(hex)
        }
    }

    pub fn from_accent(color: AccentColor) -> Self {
        let channel = |value: f64| (value * 255.0).round().clamp(0.0, 255.0) as u8;
        Self {
            red: channel(color.red()),
            green: channel(color.green()),
            blue: channel(color.blue()),
            alpha: 0xff,
        }
    }

    pub const fn hex(self) -> u32 {
        (self.red as u32) << 16 | (self.green as u32) << 8 | self.blue as u32
    }

    pub fn hsla(self) -> Hsla {
        if self.alpha == 0xff {
            rgb(self.hex()).into()
        } else {
            rgba((self.hex() << 8) | u32::from(self.alpha)).into()
        }
    }

    fn relative_luminance(self) -> f64 {
        let channel = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(self.red) + 0.7152 * channel(self.green) + 0.0722 * channel(self.blue)
    }

    pub fn contrast_ratio(self, other: Self) -> f64 {
        let a = self.relative_luminance();
        let b = other.relative_luminance();
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorTokens {
    pub window: RgbaColor,
    pub chrome: RgbaColor,
    pub sidebar: RgbaColor,
    pub list: RgbaColor,
    pub raised: RgbaColor,
    pub text: RgbaColor,
    pub text_secondary: RgbaColor,
    pub text_tertiary: RgbaColor,
    pub separator: RgbaColor,
    pub hover: RgbaColor,
    pub selection_unfocused: RgbaColor,
    pub row_alternate: RgbaColor,
    pub control_fill: RgbaColor,
    pub control_fill_hover: RgbaColor,
    pub accent: RgbaColor,
    pub accent_subtle: RgbaColor,
    pub accent_border: RgbaColor,
    pub on_accent: RgbaColor,
    pub danger: RgbaColor,
    pub on_danger: RgbaColor,
    pub error_background: RgbaColor,
    pub error_border: RgbaColor,
    pub warning_background: RgbaColor,
    pub warning_border: RgbaColor,
    pub warning_text: RgbaColor,
    pub scrim: RgbaColor,
    pub notes_accent: RgbaColor,
    pub notes_selection: RgbaColor,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypographyTokens {
    pub body: f32,
    pub callout: f32,
    pub caption: f32,
    pub headline: f32,
    pub title: f32,
    pub regular: FontWeight,
    pub medium: FontWeight,
    pub semibold: FontWeight,
    pub bold: FontWeight,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpacingTokens {
    pub x1: f32,
    pub x2: f32,
    pub x3: f32,
    pub x4: f32,
    pub x5: f32,
    pub x6: f32,
    pub x8: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadiusTokens {
    pub control: f32,
    pub card: f32,
    pub popover: f32,
    pub large_surface: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FocusTokens {
    pub ring_width: f32,
    pub ring_offset: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElevationLevel {
    pub shadow_alpha: f32,
    pub blur: f32,
    pub offset_y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElevationTokens {
    pub content: ElevationLevel,
    pub raised: ElevationLevel,
    pub popover: ElevationLevel,
    pub modal: ElevationLevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MotionTokens {
    pub fast_ms: u16,
    pub standard_ms: u16,
    pub deliberate_ms: u16,
    pub spatial_motion: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeTokens {
    pub color_scheme: ResolvedColorScheme,
    pub colors: ColorTokens,
    pub typography: TypographyTokens,
    pub spacing: SpacingTokens,
    pub radii: RadiusTokens,
    pub focus: FocusTokens,
    pub elevation: ElevationTokens,
    pub motion: MotionTokens,
    pub text_scale: TextScale,
}

impl ThemeTokens {
    pub fn from_appearance(appearance: ResolvedAppearance) -> Self {
        let accent = RgbaColor::from_accent(appearance.accent_color);
        let white = RgbaColor::opaque(0xffffff);
        let black = RgbaColor::opaque(0x000000);
        let on_accent = if accent.contrast_ratio(white) >= accent.contrast_ratio(black) {
            white
        } else {
            black
        };
        let high_contrast = appearance.contrast == Contrast::Higher;
        let colors = match appearance.color_scheme {
            ResolvedColorScheme::Light => ColorTokens {
                window: RgbaColor::opaque(0xffffff),
                chrome: RgbaColor::opaque(0xf6f6f6),
                sidebar: RgbaColor::opaque(0xf2f2f2),
                list: RgbaColor::opaque(0xffffff),
                raised: RgbaColor::opaque(0xffffff),
                text: RgbaColor::opaque(0x1d1d1f),
                text_secondary: RgbaColor::opaque(if high_contrast { 0x48484d } else { 0x66666c }),
                text_tertiary: RgbaColor::opaque(if high_contrast { 0x5a5a60 } else { 0x6e6e73 }),
                separator: RgbaColor::with_alpha(0x000000, if high_contrast { 0x42 } else { 0x18 }),
                hover: RgbaColor::with_alpha(0x000000, if high_contrast { 0x14 } else { 0x0a }),
                selection_unfocused: RgbaColor::with_alpha(
                    0x000000,
                    if high_contrast { 0x28 } else { 0x14 },
                ),
                row_alternate: RgbaColor::opaque(0xf4f5f5),
                control_fill: RgbaColor::opaque(0xe9e9ec),
                control_fill_hover: RgbaColor::opaque(0xdedee2),
                accent,
                accent_subtle: RgbaColor {
                    alpha: 0x22,
                    ..accent
                },
                accent_border: RgbaColor {
                    alpha: 0x66,
                    ..accent
                },
                on_accent,
                danger: RgbaColor::opaque(0xd70015),
                on_danger: RgbaColor::opaque(0xffffff),
                error_background: RgbaColor::with_alpha(0xd70015, 0x18),
                error_border: RgbaColor::with_alpha(0xd70015, 0x55),
                warning_background: RgbaColor::opaque(0xfff6da),
                warning_border: RgbaColor::opaque(0xeedca0),
                warning_text: RgbaColor::opaque(0x7a5c00),
                scrim: RgbaColor::with_alpha(0x000000, 0x38),
                notes_accent: RgbaColor::opaque(0xffc40c),
                notes_selection: RgbaColor::opaque(0xfdeaa3),
            },
            ResolvedColorScheme::Dark => ColorTokens {
                window: RgbaColor::opaque(0x1e1e20),
                chrome: RgbaColor::opaque(0x29292c),
                sidebar: RgbaColor::opaque(0x242426),
                list: RgbaColor::opaque(0x1e1e20),
                raised: RgbaColor::opaque(0x323236),
                text: RgbaColor::opaque(0xf5f5f7),
                text_secondary: RgbaColor::opaque(if high_contrast { 0xd4d4d8 } else { 0xb9b9bf }),
                text_tertiary: RgbaColor::opaque(if high_contrast { 0xb8b8bd } else { 0x98989f }),
                separator: RgbaColor::with_alpha(0xffffff, if high_contrast { 0x4d } else { 0x20 }),
                hover: RgbaColor::with_alpha(0xffffff, if high_contrast { 0x18 } else { 0x0e }),
                selection_unfocused: RgbaColor::with_alpha(
                    0xffffff,
                    if high_contrast { 0x30 } else { 0x1c },
                ),
                row_alternate: RgbaColor::opaque(0x242427),
                control_fill: RgbaColor::opaque(0x3a3a3e),
                control_fill_hover: RgbaColor::opaque(0x4a4a4f),
                accent,
                accent_subtle: RgbaColor {
                    alpha: 0x30,
                    ..accent
                },
                accent_border: RgbaColor {
                    alpha: 0x80,
                    ..accent
                },
                on_accent,
                danger: RgbaColor::opaque(0xff6961),
                on_danger: RgbaColor::opaque(0x000000),
                error_background: RgbaColor::with_alpha(0xff6961, 0x24),
                error_border: RgbaColor::with_alpha(0xff6961, 0x70),
                warning_background: RgbaColor::opaque(0x3a321e),
                warning_border: RgbaColor::opaque(0x756225),
                warning_text: RgbaColor::opaque(0xffd76a),
                scrim: RgbaColor::with_alpha(0x000000, 0x70),
                notes_accent: RgbaColor::opaque(0xffd60a),
                notes_selection: RgbaColor::opaque(0x5c4b08),
            },
        };
        let text_factor = appearance.text_scale.factor();
        Self {
            color_scheme: appearance.color_scheme,
            colors,
            typography: TypographyTokens {
                body: 13.0 * text_factor,
                callout: 12.0 * text_factor,
                caption: 11.0 * text_factor,
                headline: 15.0 * text_factor,
                title: 22.0 * text_factor,
                regular: FontWeight::NORMAL,
                medium: FontWeight::MEDIUM,
                semibold: FontWeight::SEMIBOLD,
                bold: FontWeight::BOLD,
            },
            spacing: SpacingTokens {
                x1: 4.0,
                x2: 8.0,
                x3: 12.0,
                x4: 16.0,
                x5: 20.0,
                x6: 24.0,
                x8: 32.0,
            },
            radii: RadiusTokens {
                control: 6.0,
                card: 10.0,
                popover: 12.0,
                large_surface: 16.0,
            },
            focus: FocusTokens {
                ring_width: if high_contrast { 3.0 } else { 2.0 },
                ring_offset: 2.0,
            },
            elevation: ElevationTokens {
                content: ElevationLevel {
                    shadow_alpha: 0.0,
                    blur: 0.0,
                    offset_y: 0.0,
                },
                raised: ElevationLevel {
                    shadow_alpha: if high_contrast { 0.18 } else { 0.10 },
                    blur: 8.0,
                    offset_y: 2.0,
                },
                popover: ElevationLevel {
                    shadow_alpha: if high_contrast { 0.28 } else { 0.18 },
                    blur: 18.0,
                    offset_y: 6.0,
                },
                modal: ElevationLevel {
                    shadow_alpha: if high_contrast { 0.36 } else { 0.24 },
                    blur: 28.0,
                    offset_y: 10.0,
                },
            },
            motion: if appearance.motion == MotionPreference::Reduced {
                MotionTokens {
                    fast_ms: 80,
                    standard_ms: 100,
                    deliberate_ms: 120,
                    spatial_motion: false,
                }
            } else {
                MotionTokens {
                    fast_ms: 120,
                    standard_ms: 180,
                    deliberate_ms: 240,
                    spatial_motion: true,
                }
            },
            text_scale: appearance.text_scale,
        }
    }

    pub fn light_default() -> Self {
        Self::from_appearance(ResolvedAppearance {
            color_scheme: ResolvedColorScheme::Light,
            accent_color: AccentColor::new(0.0, 122.0 / 255.0, 1.0)
                .expect("default accent is valid"),
            contrast: Contrast::Normal,
            motion: MotionPreference::Full,
            text_scale: TextScale::Standard,
        })
    }
}

pub fn current() -> ThemeTokens {
    *CURRENT_TOKENS
        .get_or_init(|| RwLock::new(ThemeTokens::light_default()))
        .read()
        .expect("theme token lock poisoned")
}

pub(crate) fn set_current(tokens: ThemeTokens) -> bool {
    let mut current = CURRENT_TOKENS
        .get_or_init(|| RwLock::new(ThemeTokens::light_default()))
        .write()
        .expect("theme token lock poisoned");
    if *current == tokens {
        false
    } else {
        *current = tokens;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn appearance(
        scheme: ResolvedColorScheme,
        accent: (f64, f64, f64),
        contrast: Contrast,
        motion: MotionPreference,
    ) -> ResolvedAppearance {
        ResolvedAppearance {
            color_scheme: scheme,
            accent_color: AccentColor::new(accent.0, accent.1, accent.2).unwrap(),
            contrast,
            motion,
            text_scale: TextScale::Standard,
        }
    }

    #[test]
    fn primary_and_secondary_text_meet_normal_text_contrast() {
        for scheme in [ResolvedColorScheme::Light, ResolvedColorScheme::Dark] {
            let colors = ThemeTokens::from_appearance(appearance(
                scheme,
                (0.0, 0.48, 1.0),
                Contrast::Normal,
                MotionPreference::Full,
            ))
            .colors;
            assert!(colors.text.contrast_ratio(colors.window) >= 4.5);
            assert!(colors.text_secondary.contrast_ratio(colors.window) >= 4.5);
            assert!(colors.text_tertiary.contrast_ratio(colors.window) >= 4.5);
            assert!(colors.on_danger.contrast_ratio(colors.danger) >= 4.5);
        }
    }

    #[test]
    fn on_accent_chooses_the_more_legible_black_or_white() {
        for accent in [(1.0, 0.8, 0.0), (0.0, 0.2, 0.5)] {
            let colors = ThemeTokens::from_appearance(appearance(
                ResolvedColorScheme::Light,
                accent,
                Contrast::Normal,
                MotionPreference::Full,
            ))
            .colors;
            assert!(colors.on_accent.contrast_ratio(colors.accent) >= 4.5);
        }
    }

    #[test]
    fn text_scale_changes_semantic_typography_without_layout_spacing() {
        let standard = ThemeTokens::from_appearance(appearance(
            ResolvedColorScheme::Light,
            (0.0, 0.48, 1.0),
            Contrast::Normal,
            MotionPreference::Full,
        ));
        let mut larger_appearance = appearance(
            ResolvedColorScheme::Light,
            (0.0, 0.48, 1.0),
            Contrast::Normal,
            MotionPreference::Full,
        );
        larger_appearance.text_scale = TextScale::ExtraLarge;
        let larger = ThemeTokens::from_appearance(larger_appearance);
        assert!(larger.typography.body > standard.typography.body);
        assert!(larger.typography.caption > standard.typography.caption);
        assert_eq!(larger.spacing, standard.spacing);
        assert_eq!(larger.text_scale, TextScale::ExtraLarge);
    }

    #[test]
    fn increased_contrast_strengthens_non_text_boundaries_and_focus() {
        let normal = ThemeTokens::from_appearance(appearance(
            ResolvedColorScheme::Light,
            (0.0, 0.48, 1.0),
            Contrast::Normal,
            MotionPreference::Full,
        ));
        let high = ThemeTokens::from_appearance(appearance(
            ResolvedColorScheme::Light,
            (0.0, 0.48, 1.0),
            Contrast::Higher,
            MotionPreference::Full,
        ));
        assert!(high.colors.separator.alpha > normal.colors.separator.alpha);
        assert!(high.focus.ring_width > normal.focus.ring_width);
    }

    #[test]
    fn spacing_uses_the_four_point_rhythm() {
        let spacing = ThemeTokens::light_default().spacing;
        for value in [
            spacing.x1, spacing.x2, spacing.x3, spacing.x4, spacing.x5, spacing.x6, spacing.x8,
        ] {
            assert_eq!(value % 4.0, 0.0);
        }
    }

    #[test]
    fn reduced_motion_disables_spatial_transitions() {
        let full = ThemeTokens::light_default().motion;
        let reduced = ThemeTokens::from_appearance(appearance(
            ResolvedColorScheme::Light,
            (0.0, 0.48, 1.0),
            Contrast::Normal,
            MotionPreference::Reduced,
        ))
        .motion;
        assert!(full.spatial_motion);
        assert!(!reduced.spatial_motion);
        assert!(reduced.standard_ms < full.standard_ms);
    }
}
