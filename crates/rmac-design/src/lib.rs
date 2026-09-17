//! The single source of design tokens for every rmac surface.
//!
//! This crate is deliberately GPUI-free: it holds plain data (`Rgba(u32)`,
//! `f32`) so both the stable-GPUI application workspace (`rmac-ui`) and the
//! upstream-GPUI shell workspace (`shell/crates/rmac-shell-ui`) convert from
//! the same values. Nothing else in the product hard-codes a color, radius,
//! size, duration, or font size.

mod color;
mod elevation;
mod geometry;
mod material;
mod motion;
mod typography;

pub use color::{Colors, Rgba};
pub use elevation::{Elevation, ElevationLevel, WindowShadow};
pub use geometry::{Metrics, Radii, Spacing};
pub use material::{Material, Materials};
pub use motion::{Curve, Motion, MotionSpec};
pub use typography::{TextStyle, TypeScale, Weight};

use rmac_appearance::{
    AccentColor, Contrast, MotionPreference, ResolvedAppearance, ResolvedColorScheme, TextScale,
};

/// The product UI font. Declared as the `fonts-inter` package dependency.
pub const UI_FONT: &str = "Inter";
/// The product monospace font, from `fonts-jetbrains-mono`.
pub const MONO_FONT: &str = "JetBrains Mono";

/// Every resolved design token for one appearance snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub color_scheme: ResolvedColorScheme,
    pub contrast: Contrast,
    pub text_scale: TextScale,
    pub colors: Colors,
    pub materials: Materials,
    pub type_scale: TypeScale,
    pub spacing: Spacing,
    pub radii: Radii,
    pub metrics: Metrics,
    pub elevation: Elevation,
    pub motion: Motion,
}

impl Tokens {
    /// Resolve all tokens from the live appearance authority.
    pub fn resolve(appearance: ResolvedAppearance) -> Self {
        let colors = Colors::resolve(
            appearance.color_scheme,
            appearance.accent_color,
            appearance.contrast,
        );
        let materials = Materials::resolve(appearance.color_scheme, appearance.contrast, colors);
        let mut metrics = Metrics::default();
        if appearance.contrast == Contrast::Higher {
            metrics.focus_ring_width = metrics.focus_ring_width_high_contrast;
        }
        Self {
            color_scheme: appearance.color_scheme,
            contrast: appearance.contrast,
            text_scale: appearance.text_scale,
            colors,
            materials,
            type_scale: TypeScale::resolve(appearance.text_scale),
            spacing: Spacing::default(),
            radii: Radii::default(),
            metrics,
            elevation: Elevation::resolve(appearance.contrast),
            motion: Motion::resolve(appearance.motion),
        }
    }

    /// The default light appearance, for tests and first paint.
    pub fn light_default() -> Self {
        Self::resolve(ResolvedAppearance {
            color_scheme: ResolvedColorScheme::Light,
            accent_color: AccentColor::new(0.0, 122.0 / 255.0, 1.0)
                .expect("default accent is valid"),
            contrast: Contrast::Normal,
            motion: MotionPreference::Full,
            text_scale: TextScale::Standard,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn appearance(scheme: ResolvedColorScheme, contrast: Contrast) -> ResolvedAppearance {
        ResolvedAppearance {
            color_scheme: scheme,
            accent_color: AccentColor::new(0.0, 0.48, 1.0).unwrap(),
            contrast,
            motion: MotionPreference::Full,
            text_scale: TextScale::Standard,
        }
    }

    #[test]
    fn primary_text_meets_aa_and_secondary_meets_large_text_contrast() {
        for scheme in [ResolvedColorScheme::Light, ResolvedColorScheme::Dark] {
            for contrast in [Contrast::Normal, Contrast::Higher] {
                let colors = Tokens::resolve(appearance(scheme, contrast)).colors;
                assert!(
                    colors.label_primary.contrast_ratio(colors.surface_window) >= 4.5,
                    "primary contrast failed for {scheme:?}/{contrast:?}"
                );
                assert!(
                    colors.label_secondary.contrast_ratio(colors.surface_window) >= 3.0,
                    "secondary contrast failed for {scheme:?}/{contrast:?}"
                );
            }
        }
    }

    #[test]
    fn reduced_transparency_fallbacks_are_effectively_opaque() {
        for scheme in [ResolvedColorScheme::Light, ResolvedColorScheme::Dark] {
            let tokens = Tokens::resolve(appearance(scheme, Contrast::Normal));
            for material in [
                tokens.materials.menu,
                tokens.materials.popover,
                tokens.materials.hud,
                tokens.materials.dock,
                tokens.materials.sidebar,
                tokens.materials.menubar,
                tokens.materials.tooltip,
            ] {
                assert!(
                    material.fallback.alpha() >= 0xf0,
                    "fallback not opaque enough for {scheme:?}"
                );
            }
        }
    }

    #[test]
    fn on_accent_stays_legible_for_light_and_dark_accents() {
        for accent in [(1.0, 0.8, 0.0), (0.0, 0.2, 0.5)] {
            let mut resolved = appearance(ResolvedColorScheme::Light, Contrast::Normal);
            resolved.accent_color = AccentColor::new(accent.0, accent.1, accent.2).unwrap();
            let colors = Tokens::resolve(resolved).colors;
            assert!(colors.on_accent.contrast_ratio(colors.accent) >= 4.5);
        }
    }

    #[test]
    fn increased_contrast_strengthens_boundaries_and_materials() {
        let normal = Tokens::resolve(appearance(ResolvedColorScheme::Light, Contrast::Normal));
        let high = Tokens::resolve(appearance(ResolvedColorScheme::Light, Contrast::Higher));
        assert!(high.colors.separator.alpha() > normal.colors.separator.alpha());
        assert!(high.metrics.focus_ring_width > normal.metrics.focus_ring_width);
        assert!(high.materials.menu.tint.alpha() > normal.materials.menu.tint.alpha());
        assert!(high.elevation.raised.alpha > normal.elevation.raised.alpha);
    }

    #[test]
    fn text_scale_scales_type_without_touching_geometry() {
        let standard = Tokens::light_default();
        let mut larger = appearance(ResolvedColorScheme::Light, Contrast::Normal);
        larger.text_scale = TextScale::ExtraLarge;
        let larger = Tokens::resolve(larger);
        assert!(larger.type_scale.body.size > standard.type_scale.body.size);
        assert!(larger.type_scale.lock_time.size > standard.type_scale.lock_time.size);
        assert_eq!(larger.spacing, standard.spacing);
        assert_eq!(larger.metrics, standard.metrics);
    }

    #[test]
    fn reduced_motion_is_recorded_on_the_tokens() {
        let mut reduced = appearance(ResolvedColorScheme::Light, Contrast::Normal);
        reduced.motion = MotionPreference::Reduced;
        let tokens = Tokens::resolve(reduced);
        assert!(tokens.motion.reduced_motion);
        assert!(!Tokens::light_default().motion.reduced_motion);
        assert!(tokens.motion.minimize_reduce_fade_ms < tokens.motion.minimize_ms);
    }

    #[test]
    fn accent_palette_offers_the_nine_appearance_choices() {
        let palette = Tokens::light_default().colors.accent_palette();
        assert_eq!(palette.len(), 9);
        assert_eq!(palette[0].0, "Multicolor");
        assert_eq!(palette[0].1, Tokens::light_default().colors.system_blue);
        assert_eq!(palette[8].0, "Graphite");
    }

    #[test]
    fn spacing_uses_the_four_point_rhythm() {
        let spacing = Tokens::light_default().spacing;
        for value in [
            spacing.x1, spacing.x2, spacing.x3, spacing.x4, spacing.x5, spacing.x6, spacing.x8,
        ] {
            assert_eq!(value % 4.0, 0.0);
        }
    }
}
