//! Semantic design tokens derived from one resolved appearance snapshot.
//!
//! The values come from [`rmac_design`], the single token source shared with
//! the shell workspace. This module keeps the stable app-facing API names and
//! adds the few app-only derivations (subtle accent fills, error surfaces)
//! that are not standalone design tokens.

use gpui::{rgb, rgba, FontWeight, Hsla};
use rmac_appearance::{
    AccentColor, Contrast, MotionPreference, ResolvedAppearance, ResolvedColorScheme, TextScale,
};
use rmac_design::{Rgba, Tokens as DesignTokens};
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

    pub const fn with_opacity(self, alpha: u8) -> Self {
        Self { alpha, ..self }
    }

    pub fn from_accent(color: AccentColor) -> Self {
        Rgba::from_accent(color).into()
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

impl From<Rgba> for RgbaColor {
    fn from(color: Rgba) -> Self {
        Self {
            red: color.red(),
            green: color.green(),
            blue: color.blue(),
            alpha: color.alpha(),
        }
    }
}

/// The effective duration of a motion token under the user's motion setting.
fn motion_duration(spec: rmac_design::MotionSpec, reduced: bool) -> u16 {
    if reduced {
        spec.reduce_duration_ms
    } else {
        spec.duration_ms
    }
}

/// Picks whichever of white or black is more legible on `background`.
fn on_color(background: RgbaColor) -> RgbaColor {
    let white = RgbaColor::opaque(0xffffff);
    let black = RgbaColor::opaque(0x000000);
    if background.contrast_ratio(white) >= background.contrast_ratio(black) {
        white
    } else {
        black
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
    pub white: RgbaColor,
    pub black: RgbaColor,
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
    pub pill: f32,
}

/// Adaptive material tints for the distinct macOS visual layers.
///
/// Content remains opaque. Navigation and transient controls may let the
/// compositor-provided background blur show through, while a clear material is
/// reserved for small controls over visually rich content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialTokens {
    pub content: RgbaColor,
    pub regular: RgbaColor,
    pub clear: RgbaColor,
    pub sidebar: RgbaColor,
    pub hud: RgbaColor,
}

/// Shared component geometry in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComponentMetricsTokens {
    pub compact_control_height: f32,
    pub regular_control_height: f32,
    pub toolbar_height: f32,
    pub sidebar_row_height: f32,
    pub list_row_height: f32,
    pub toggle_width: f32,
    pub toggle_height: f32,
    pub toggle_thumb: f32,
    pub switch_regular_width: f32,
    pub switch_regular_height: f32,
    pub switch_regular_thumb: f32,
    pub switch_small_width: f32,
    pub switch_small_height: f32,
    pub switch_small_thumb: f32,
    pub switch_mini_width: f32,
    pub switch_mini_height: f32,
    pub switch_mini_thumb: f32,
    pub traffic_light_hit_width: f32,
    pub traffic_light_hit_height: f32,
    pub traffic_light_diameter: f32,
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

impl From<rmac_design::ElevationLevel> for ElevationLevel {
    fn from(level: rmac_design::ElevationLevel) -> Self {
        Self {
            shadow_alpha: level.alpha,
            blur: level.blur,
            offset_y: level.offset_y,
        }
    }
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
    pub materials: MaterialTokens,
    pub metrics: ComponentMetricsTokens,
    pub focus: FocusTokens,
    pub elevation: ElevationTokens,
    pub motion: MotionTokens,
    pub text_scale: TextScale,
}

impl ThemeTokens {
    pub fn from_appearance(appearance: ResolvedAppearance) -> Self {
        let design = DesignTokens::resolve(appearance);
        let light = appearance.color_scheme == ResolvedColorScheme::Light;
        let accent: RgbaColor = design.colors.accent.into();
        let danger: RgbaColor = design.colors.danger.into();

        let colors = ColorTokens {
            window: design.colors.surface_window.into(),
            chrome: design.colors.surface_chrome.into(),
            sidebar: design.colors.surface_sidebar_opaque.into(),
            list: design.colors.surface_window.into(),
            raised: design.colors.surface_raised.into(),
            text: design.colors.label_primary.into(),
            text_secondary: design.colors.label_secondary.into(),
            text_tertiary: design.colors.label_tertiary.into(),
            separator: design.colors.separator.into(),
            hover: design.colors.fill_hover.into(),
            selection_unfocused: design.colors.selection_unfocused.into(),
            row_alternate: design.colors.row_alternate.into(),
            control_fill: design.colors.fill_control.into(),
            control_fill_hover: design.colors.fill_control_hover.into(),
            accent,
            accent_subtle: accent.with_opacity(if light { 0x22 } else { 0x30 }),
            accent_border: accent.with_opacity(if light { 0x66 } else { 0x80 }),
            on_accent: design.colors.on_accent.into(),
            white: design.colors.white.into(),
            black: design.colors.black.into(),
            danger,
            on_danger: on_color(danger),
            error_background: danger.with_opacity(if light { 0x18 } else { 0x24 }),
            error_border: danger.with_opacity(if light { 0x55 } else { 0x70 }),
            warning_background: design.colors.warning_background.into(),
            warning_border: design.colors.warning_border.into(),
            warning_text: design.colors.warning_text.into(),
            scrim: design.colors.scrim.into(),
            notes_accent: design.colors.notes_accent.into(),
            notes_selection: RgbaColor::opaque(if light { 0xfdeaa3 } else { 0x5c4b08 }),
        };

        let type_scale = &design.type_scale;
        let typography = TypographyTokens {
            body: type_scale.body.size,
            callout: type_scale.callout.size,
            caption: type_scale.subheadline.size,
            headline: type_scale.title3.size,
            title: type_scale.title1.size,
            regular: FontWeight::NORMAL,
            medium: FontWeight::MEDIUM,
            semibold: FontWeight::SEMIBOLD,
            bold: FontWeight::BOLD,
        };

        let radii = RadiusTokens {
            control: design.radii.control,
            card: design.radii.card,
            popover: design.radii.popover,
            large_surface: design.radii.large,
            pill: design.radii.pill,
        };

        let metrics = ComponentMetricsTokens {
            compact_control_height: design.metrics.control_height_regular,
            regular_control_height: design.metrics.control_height_large,
            toolbar_height: design.metrics.toolbar_height,
            sidebar_row_height: design.metrics.sidebar_row_height,
            list_row_height: design.metrics.list_row_height_regular,
            toggle_width: design.metrics.switch_regular_width,
            toggle_height: design.metrics.switch_regular_height,
            toggle_thumb: design.metrics.switch_regular_thumb,
            switch_regular_width: design.metrics.switch_regular_width,
            switch_regular_height: design.metrics.switch_regular_height,
            switch_regular_thumb: design.metrics.switch_regular_thumb,
            switch_small_width: design.metrics.switch_small_width,
            switch_small_height: design.metrics.switch_small_height,
            switch_small_thumb: design.metrics.switch_small_thumb,
            switch_mini_width: design.metrics.switch_mini_width,
            switch_mini_height: design.metrics.switch_mini_height,
            switch_mini_thumb: design.metrics.switch_mini_thumb,
            traffic_light_hit_width: design.metrics.traffic_spacing,
            traffic_light_hit_height: design.metrics.hit_target_min,
            traffic_light_diameter: design.metrics.traffic_diameter,
        };

        Self {
            color_scheme: appearance.color_scheme,
            colors,
            typography,
            spacing: SpacingTokens {
                x1: design.spacing.x1,
                x2: design.spacing.x2,
                x3: design.spacing.x3,
                x4: design.spacing.x4,
                x5: design.spacing.x5,
                x6: design.spacing.x6,
                x8: design.spacing.x8,
            },
            radii,
            materials: MaterialTokens {
                content: design.colors.surface_window.into(),
                regular: design.materials.menu.tint.into(),
                clear: design.materials.dock.tint.into(),
                sidebar: design.materials.sidebar.tint.into(),
                hud: design.materials.hud.tint.into(),
            },
            metrics,
            focus: FocusTokens {
                ring_width: design.metrics.focus_ring_width,
                ring_offset: 2.0,
            },
            elevation: ElevationTokens {
                content: design.elevation.content.into(),
                raised: design.elevation.raised.into(),
                popover: design.elevation.popover.into(),
                modal: design.elevation.modal.into(),
            },
            motion: MotionTokens {
                fast_ms: motion_duration(design.motion.fast, design.motion.reduced_motion),
                standard_ms: motion_duration(design.motion.standard, design.motion.reduced_motion),
                deliberate_ms: motion_duration(
                    design.motion.deliberate,
                    design.motion.reduced_motion,
                ),
                spatial_motion: !design.motion.reduced_motion,
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
    use rmac_appearance::{AccentColor, MotionPreference};

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
            assert!(colors.text_secondary.contrast_ratio(colors.window) >= 3.0);
            assert!(colors.text_tertiary.contrast_ratio(colors.window) >= 3.0);
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
        assert!(high.materials.regular.alpha > normal.materials.regular.alpha);
        assert!(high.materials.clear.alpha > normal.materials.clear.alpha);
    }

    #[test]
    fn materials_keep_content_opaque_and_glass_roles_ordered() {
        for scheme in [ResolvedColorScheme::Light, ResolvedColorScheme::Dark] {
            let tokens = ThemeTokens::from_appearance(appearance(
                scheme,
                (0.0, 0.48, 1.0),
                Contrast::Normal,
                MotionPreference::Full,
            ));
            assert_eq!(tokens.materials.content.alpha, 0xff);
            assert!(tokens.materials.regular.alpha > tokens.materials.clear.alpha);
            assert!(tokens.materials.hud.alpha > tokens.materials.clear.alpha);
        }
    }

    #[test]
    fn shared_component_metrics_preserve_desktop_density() {
        let metrics = ThemeTokens::light_default().metrics;
        assert!(metrics.compact_control_height < metrics.regular_control_height);
        assert!(metrics.regular_control_height < metrics.toolbar_height);
        assert!(metrics.toggle_thumb < metrics.toggle_height);
        assert!(metrics.traffic_light_diameter < metrics.traffic_light_hit_width);
        assert_eq!(metrics.sidebar_row_height % 2.0, 0.0);
        assert_eq!(metrics.list_row_height % 2.0, 0.0);
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
