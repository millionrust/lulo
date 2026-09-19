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

pub use color::{tint, Colors, Rgba};
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
    /// macOS 27 Liquid Glass intensity, 0.0 (clear) … 1.0 (tinted). The
    /// Appearance pane drives this; 0.5 is the default mid setting.
    pub glass_intensity: f32,
    /// Dominant 8×8 sRGB wallpaper average. Transparent means unavailable.
    pub wallpaper_tint: Rgba,
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
        let glass_intensity = 0.5;
        let materials = Materials::resolve(
            appearance.color_scheme,
            appearance.contrast,
            colors,
            glass_intensity,
        );
        let mut metrics = Metrics::default();
        if appearance.contrast == Contrast::Higher {
            metrics.focus_ring_width = metrics.focus_ring_width_high_contrast;
        }
        Self {
            color_scheme: appearance.color_scheme,
            contrast: appearance.contrast,
            text_scale: appearance.text_scale,
            glass_intensity,
            wallpaper_tint: Rgba::TRANSPARENT,
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

    /// Re-resolve the materials for a Liquid Glass intensity (macOS 27).
    pub fn with_glass_intensity(mut self, glass_intensity: f32) -> Self {
        self.glass_intensity = glass_intensity.clamp(0.0, 1.0);
        self.materials = Materials::resolve(
            self.color_scheme,
            self.contrast,
            self.colors,
            self.glass_intensity,
        );
        self
    }

    /// Apply the measured wallpaper mix to app surfaces. The sidebar material
    /// derives from its explicitly tinted sidebar token; floating glass such
    /// as menus and Control Center remains unchanged because compositor blur
    /// already reveals the wallpaper there.
    pub fn with_wallpaper_tint(mut self, wallpaper_tint: Rgba) -> Self {
        self.wallpaper_tint = wallpaper_tint.with_alpha(0xff);
        self.colors.surface_window = tint(self.colors.surface_window, wallpaper_tint, 0.06);
        self.colors.surface_sidebar_opaque =
            tint(self.colors.surface_sidebar_opaque, wallpaper_tint, 0.10);
        self.colors.surface_grouped_background =
            tint(self.colors.surface_grouped_background, wallpaper_tint, 0.06);
        self.colors.surface_grouped_row =
            tint(self.colors.surface_grouped_row, wallpaper_tint, 0.07);
        self.colors.surface_sheet = tint(self.colors.surface_sheet, wallpaper_tint, 0.05);
        self.colors.surface_chrome = tint(self.colors.surface_chrome, wallpaper_tint, 0.08);
        self.materials = Materials::resolve(
            self.color_scheme,
            self.contrast,
            self.colors,
            self.glass_intensity,
        );
        self
    }

    /// The default light appearance, for tests and first paint.
    pub fn light_default() -> Self {
        Self::resolve(ResolvedAppearance {
            color_scheme: ResolvedColorScheme::Light,
            accent_color: AccentColor::new(19.0 / 255.0, 114.0 / 255.0, 249.0 / 255.0)
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
        assert!(high.materials.menu.tint.alpha() >= normal.materials.menu.tint.alpha());
        assert!(high.elevation.raised.alpha > normal.elevation.raised.alpha);
    }

    #[test]
    fn window_elevation_matches_the_design_lab_mapping() {
        let elevation =
            Tokens::resolve(appearance(ResolvedColorScheme::Dark, Contrast::Normal)).elevation;
        assert_eq!(elevation.window_active.softness, 32.0);
        assert_eq!(elevation.window_active.spread, 0.0);
        assert_eq!(elevation.window_active.offset_y, 12.0);
        assert_eq!(elevation.window_active.color, Rgba::from_rgba(0x00000073));
        assert_eq!(elevation.window_inactive.softness, 12.0);
        assert_eq!(elevation.window_inactive.spread, 0.0);
        assert_eq!(elevation.window_inactive.offset_y, 4.0);
        assert_eq!(elevation.window_inactive.color, Rgba::from_rgba(0x0000004d));
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
        assert_eq!(Tokens::light_default().colors.accent, Rgba::rgb(0x1372f9));
    }

    #[test]
    fn dark_neutral_bases_are_ready_for_runtime_wallpaper_tinting() {
        let colors =
            Tokens::resolve(appearance(ResolvedColorScheme::Dark, Contrast::Normal)).colors;
        assert_eq!(colors.surface_window, Rgba::rgb(0x1e1e20));
        assert_eq!(colors.surface_sidebar_opaque, Rgba::rgb(0x242426));
        assert_eq!(colors.surface_grouped_background, Rgba::rgb(0x1c1c1e));
        assert_eq!(colors.surface_grouped_row, Rgba::rgb(0x2c2c2e));
        assert_eq!(colors.surface_sheet, Rgba::rgb(0x2c2c2e));
        assert_eq!(colors.field_fill, Rgba::rgb(0x181818));
        assert_eq!(colors.statusbar, Rgba::rgb(0x29272c));
        assert_eq!(colors.button_secondary, Rgba::rgb(0x363237));
        assert_eq!(colors.button_destructive, Rgba::rgb(0x812e25));
        assert_eq!(colors.menubar_text, Rgba::rgb(0xffffff));
        assert_eq!(colors.system_blue, Rgba::rgb(0x1372f9));
    }

    #[test]
    fn wallpaper_tint_uses_measured_strengths_without_recoloring_floating_glass() {
        let base = Tokens::resolve(appearance(ResolvedColorScheme::Dark, Contrast::Normal));
        let tinted = base.with_wallpaper_tint(Rgba::rgb(0xc040e0));
        assert_eq!(tinted.wallpaper_tint, Rgba::rgb(0xc040e0));
        assert_eq!(tinted.colors.surface_window, Rgba::rgb(0x28202c));
        assert_eq!(tinted.colors.surface_sidebar_opaque, Rgba::rgb(0x342739));
        assert_eq!(
            tinted.colors.surface_grouped_background,
            Rgba::rgb(0x261e2a)
        );
        assert_eq!(tinted.colors.surface_grouped_row, Rgba::rgb(0x362d3a));
        assert_eq!(tinted.colors.surface_sheet, Rgba::rgb(0x332d37));
        assert_eq!(tinted.colors.surface_chrome, Rgba::rgb(0x352b3a));
        assert_eq!(tinted.materials.sidebar.tint, Rgba::from_rgba(0x342739e0));
        assert_eq!(tinted.materials.menu.tint, base.materials.menu.tint);
        assert_eq!(tinted.materials.popover.tint, base.materials.popover.tint);
        assert_eq!(tinted.materials.dock.tint, base.materials.dock.tint);
    }

    #[test]
    fn srgb_tint_clamps_strength_and_preserves_base_alpha() {
        assert_eq!(
            tint(Rgba::from_rgba(0x6496c880), Rgba::rgb(0xc83200), 0.25),
            Rgba::from_rgba(0x7d7d9680)
        );
        assert_eq!(
            tint(Rgba::rgb(0x102030), Rgba::rgb(0xf0e0d0), 2.0),
            Rgba::rgb(0xf0e0d0)
        );
    }

    #[test]
    fn light_materials_are_more_opaque_than_dark_materials() {
        let light = Tokens::resolve(appearance(ResolvedColorScheme::Light, Contrast::Normal));
        let dark = Tokens::resolve(appearance(ResolvedColorScheme::Dark, Contrast::Normal));
        assert!(light.materials.menu.tint.alpha() > dark.materials.menu.tint.alpha());
        assert!(light.materials.popover.tint.alpha() > dark.materials.popover.tint.alpha());
        assert!(light.materials.hud.tint.alpha() > dark.materials.hud.tint.alpha());
        assert!(light.materials.dock.tint.alpha() > dark.materials.dock.tint.alpha());
        assert!(light.materials.sidebar.tint.alpha() > dark.materials.sidebar.tint.alpha());
    }

    #[test]
    fn glass_intensity_scales_material_tint_alpha() {
        let base = Tokens::light_default();
        let clear = base.with_glass_intensity(0.0);
        let tinted = base.with_glass_intensity(1.0);
        assert!(clear.materials.menu.tint.alpha() < base.materials.menu.tint.alpha());
        assert!(tinted.materials.menu.tint.alpha() > base.materials.menu.tint.alpha());
        // The menu bar stays fully transparent at any intensity.
        assert_eq!(tinted.materials.menubar.tint.alpha(), 0);
        // High contrast wins over a clear glass setting.
        let high = Tokens::resolve(appearance(ResolvedColorScheme::Light, Contrast::Higher))
            .with_glass_intensity(0.0);
        assert!(high.materials.menu.tint.alpha() >= 0xf6);
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
