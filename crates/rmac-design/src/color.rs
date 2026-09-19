//! Color tokens as plain `0xRRGGBBAA` data.

use rmac_appearance::{AccentColor, Contrast, ResolvedColorScheme};

/// An sRGB color encoded as `0xRRGGBBAA`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba(pub u32);

impl Rgba {
    pub const TRANSPARENT: Self = Self(0x0000_0000);

    /// From an `0xRRGGBB` value with full alpha.
    pub const fn rgb(hex: u32) -> Self {
        Self((hex << 8) | 0xff)
    }

    /// From a full `0xRRGGBBAA` value.
    pub const fn from_rgba(value: u32) -> Self {
        Self(value)
    }

    pub const fn red(self) -> u8 {
        ((self.0 >> 24) & 0xff) as u8
    }

    pub const fn green(self) -> u8 {
        ((self.0 >> 16) & 0xff) as u8
    }

    pub const fn blue(self) -> u8 {
        ((self.0 >> 8) & 0xff) as u8
    }

    pub const fn alpha(self) -> u8 {
        (self.0 & 0xff) as u8
    }

    pub const fn with_alpha(self, alpha: u8) -> Self {
        Self((self.0 & 0xffff_ff00) | alpha as u32)
    }

    pub fn from_accent(color: AccentColor) -> Self {
        let channel = |value: f64| (value * 255.0).round().clamp(0.0, 255.0) as u32;
        Self(
            (channel(color.red()) << 24)
                | (channel(color.green()) << 16)
                | (channel(color.blue()) << 8)
                | 0xff,
        )
    }

    pub fn relative_luminance(self) -> f64 {
        let channel = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(self.red())
            + 0.7152 * channel(self.green())
            + 0.0722 * channel(self.blue())
    }

    pub fn contrast_ratio(self, other: Self) -> f64 {
        let a = self.relative_luminance();
        let b = other.relative_luminance();
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}

/// Semantic label, fill, surface, system, and traffic-light colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colors {
    pub label_primary: Rgba,
    pub label_secondary: Rgba,
    pub label_tertiary: Rgba,
    pub label_quaternary: Rgba,
    pub label_disabled: Rgba,
    pub label_placeholder: Rgba,

    pub separator: Rgba,
    pub separator_opaque: Rgba,

    pub fill_hover: Rgba,
    pub fill_control: Rgba,
    pub fill_control_hover: Rgba,
    pub fill_control_pressed: Rgba,

    pub selection_focused: Rgba,
    pub selection_unfocused: Rgba,
    pub selection_text: Rgba,
    pub row_alternate: Rgba,
    pub focus_ring: Rgba,

    pub surface_window: Rgba,
    pub surface_chrome: Rgba,
    pub surface_sidebar_opaque: Rgba,
    pub surface_raised: Rgba,
    pub surface_grouped_background: Rgba,
    pub surface_grouped_row: Rgba,
    pub surface_sheet: Rgba,
    pub field_fill: Rgba,
    pub statusbar: Rgba,
    pub button_secondary: Rgba,
    pub button_destructive: Rgba,
    pub menubar_text: Rgba,
    pub scrim: Rgba,

    pub system_blue: Rgba,
    pub system_purple: Rgba,
    pub system_pink: Rgba,
    pub system_red: Rgba,
    pub system_orange: Rgba,
    pub system_yellow: Rgba,
    pub system_green: Rgba,
    pub system_teal: Rgba,
    pub system_indigo: Rgba,
    pub system_brown: Rgba,
    pub system_gray: Rgba,

    pub danger: Rgba,
    pub warning_background: Rgba,
    pub warning_border: Rgba,
    pub warning_text: Rgba,
    pub notes_accent: Rgba,
    /// Stable folder-artwork blue, measured from AppKit `systemBlue`
    /// (Aqua `0088FF` / Dark Aqua `0091FF`); it does not follow the control
    /// accent.
    pub folder_blue: Rgba,

    pub accent: Rgba,
    pub on_accent: Rgba,
    /// Absolute white, used by switch thumbs and slider knobs in both schemes.
    pub white: Rgba,
    /// Absolute black.
    pub black: Rgba,

    pub traffic_close: Rgba,
    pub traffic_close_border: Rgba,
    pub traffic_minimize: Rgba,
    pub traffic_minimize_border: Rgba,
    pub traffic_zoom: Rgba,
    pub traffic_zoom_border: Rgba,
    pub traffic_inactive: Rgba,
    pub traffic_inactive_border: Rgba,
}

impl Colors {
    /// Resolve every color token for one scheme and contrast setting.
    ///
    /// The accent comes from the live appearance authority; `on_accent` picks
    /// whichever of black or white is more legible on it.
    pub fn resolve(
        scheme: ResolvedColorScheme,
        accent_color: AccentColor,
        contrast: Contrast,
    ) -> Self {
        let accent = Rgba::from_accent(accent_color);
        let white = Rgba::rgb(0xffffff);
        let black = Rgba::rgb(0x000000);
        let on_accent = if accent.contrast_ratio(white) >= accent.contrast_ratio(black) {
            white
        } else {
            black
        };
        let high = contrast == Contrast::Higher;
        match scheme {
            ResolvedColorScheme::Light => Self {
                label_primary: Rgba::rgb(0x1d1d1f),
                label_secondary: Rgba::rgb(if high { 0x48484d } else { 0x66666c }),
                label_tertiary: Rgba::rgb(if high { 0x5a5a60 } else { 0x6e6e73 }),
                label_quaternary: Rgba::from_rgba(0x00000040),
                label_disabled: Rgba::from_rgba(0x00000040),
                label_placeholder: Rgba::from_rgba(0x0000004d),

                separator: Rgba::from_rgba(if high { 0x00000042 } else { 0x00000018 }),
                separator_opaque: Rgba::rgb(0xe5e5e5),

                fill_hover: Rgba::from_rgba(if high { 0x00000014 } else { 0x0000000a }),
                fill_control: Rgba::rgb(0xe9e9ec),
                fill_control_hover: Rgba::rgb(0xdedee2),
                fill_control_pressed: Rgba::rgb(0xd2d2d7),

                selection_focused: accent,
                selection_unfocused: Rgba::from_rgba(if high { 0x00000028 } else { 0x00000014 }),
                selection_text: accent.with_alpha(0x40),
                row_alternate: Rgba::rgb(0xf4f5f5),
                focus_ring: accent.with_alpha(0x80),

                surface_window: Rgba::rgb(0xffffff),
                surface_chrome: Rgba::rgb(0xf6f6f6),
                surface_sidebar_opaque: Rgba::rgb(0xf2f2f2),
                surface_raised: Rgba::rgb(0xffffff),
                surface_grouped_background: Rgba::rgb(0xf5f5f7),
                surface_grouped_row: Rgba::rgb(0xffffff),
                // S: the light variants still need same-surface reference
                // captures; keep them isolated from the measured dark set.
                surface_sheet: Rgba::rgb(0xffffff),
                field_fill: Rgba::rgb(0xe9e9ec),
                statusbar: Rgba::rgb(0xf6f6f6),
                button_secondary: Rgba::rgb(0xe9e9ec),
                button_destructive: Rgba::rgb(0xd70015),
                menubar_text: Rgba::rgb(0x010206),
                scrim: Rgba::from_rgba(0x00000038),

                system_blue: Rgba::rgb(0x1372f9),
                system_purple: Rgba::rgb(0xaf52de),
                system_pink: Rgba::rgb(0xff2d55),
                system_red: Rgba::rgb(0xff3b30),
                system_orange: Rgba::rgb(0xff9500),
                system_yellow: Rgba::rgb(0xffcc00),
                system_green: Rgba::rgb(0x34c759),
                system_teal: Rgba::rgb(0x30b0c7),
                system_indigo: Rgba::rgb(0x5856d6),
                system_brown: Rgba::rgb(0xa2845e),
                system_gray: Rgba::rgb(0x8e8e93),

                danger: Rgba::rgb(0xd70015),
                warning_background: Rgba::rgb(0xfff6da),
                warning_border: Rgba::rgb(0xeedca0),
                warning_text: Rgba::rgb(0x7a5c00),
                notes_accent: Rgba::rgb(0xffc40c),
                folder_blue: Rgba::rgb(0x0088ff),

                accent,
                on_accent,
                white: Rgba::rgb(0xffffff),
                black: Rgba::rgb(0x000000),

                traffic_close: Rgba::rgb(0xff5f57),
                traffic_close_border: Rgba::rgb(0xe0443e),
                traffic_minimize: Rgba::rgb(0xfebc2e),
                traffic_minimize_border: Rgba::rgb(0xdea123),
                traffic_zoom: Rgba::rgb(0x28c840),
                traffic_zoom_border: Rgba::rgb(0x1aab29),
                traffic_inactive: Rgba::rgb(0xdddddd),
                traffic_inactive_border: Rgba::rgb(0xc8c8c8),
            },
            ResolvedColorScheme::Dark => Self {
                label_primary: Rgba::rgb(0xf5f5f7),
                label_secondary: Rgba::rgb(if high { 0xd4d4d8 } else { 0xb9b9bf }),
                label_tertiary: Rgba::rgb(if high { 0xb8b8bd } else { 0x98989f }),
                label_quaternary: Rgba::from_rgba(0xffffff40),
                label_disabled: Rgba::from_rgba(0xffffff40),
                label_placeholder: Rgba::from_rgba(0xffffff4d),

                separator: Rgba::from_rgba(if high { 0xffffff4d } else { 0xffffff20 }),
                separator_opaque: Rgba::rgb(0x38383a),

                fill_hover: Rgba::from_rgba(if high { 0xffffff18 } else { 0xffffff0e }),
                fill_control: Rgba::rgb(0x3a3a3e),
                fill_control_hover: Rgba::rgb(0x4a4a4f),
                fill_control_pressed: Rgba::rgb(0x55555a),

                selection_focused: accent,
                selection_unfocused: Rgba::from_rgba(if high { 0xffffff30 } else { 0xffffff1c }),
                selection_text: accent.with_alpha(0x55),
                row_alternate: Rgba::rgb(0x242427),
                focus_ring: accent.with_alpha(0x99),

                surface_window: Rgba::rgb(0x222025),
                surface_chrome: Rgba::rgb(0x29272c),
                surface_sidebar_opaque: Rgba::rgb(0x29252e),
                surface_raised: Rgba::rgb(0x323236),
                surface_grouped_background: Rgba::rgb(0x222026),
                surface_grouped_row: Rgba::rgb(0x29272d),
                surface_sheet: Rgba::rgb(0x262227),
                field_fill: Rgba::rgb(0x181818),
                statusbar: Rgba::rgb(0x29272c),
                button_secondary: Rgba::rgb(0x363237),
                button_destructive: Rgba::rgb(0x812e25),
                menubar_text: Rgba::rgb(0xffffff),
                scrim: Rgba::from_rgba(0x00000070),

                system_blue: Rgba::rgb(0x1372f9),
                system_purple: Rgba::rgb(0xbf5af2),
                system_pink: Rgba::rgb(0xff375f),
                system_red: Rgba::rgb(0xff453a),
                system_orange: Rgba::rgb(0xff9f0a),
                system_yellow: Rgba::rgb(0xffd60a),
                system_green: Rgba::rgb(0x30d158),
                system_teal: Rgba::rgb(0x40c8e0),
                system_indigo: Rgba::rgb(0x5e5ce6),
                system_brown: Rgba::rgb(0xac8e68),
                system_gray: Rgba::rgb(0x8e8e93),

                danger: Rgba::rgb(0xff6961),
                warning_background: Rgba::rgb(0x3a321e),
                warning_border: Rgba::rgb(0x756225),
                warning_text: Rgba::rgb(0xffd76a),
                notes_accent: Rgba::rgb(0xffd60a),
                folder_blue: Rgba::rgb(0x0091ff),

                accent,
                on_accent,
                white: Rgba::rgb(0xffffff),
                black: Rgba::rgb(0x000000),

                traffic_close: Rgba::rgb(0xff5f57),
                traffic_close_border: Rgba::rgb(0xe0443e),
                traffic_minimize: Rgba::rgb(0xfebc2e),
                traffic_minimize_border: Rgba::rgb(0xdea123),
                traffic_zoom: Rgba::rgb(0x28c840),
                traffic_zoom_border: Rgba::rgb(0x1aab29),
                traffic_inactive: Rgba::rgb(0x4e4e50),
                traffic_inactive_border: Rgba::rgb(0x3e3e40),
            },
        }
    }

    /// The fixed system palette offered in Appearance, in menu order.
    pub fn accent_palette(self) -> [(&'static str, Rgba); 9] {
        [
            ("Multicolor", self.system_blue),
            ("Blue", self.system_blue),
            ("Purple", self.system_purple),
            ("Pink", self.system_pink),
            ("Red", self.system_red),
            ("Orange", self.system_orange),
            ("Yellow", self.system_yellow),
            ("Green", self.system_green),
            ("Graphite", self.system_gray),
        ]
    }
}
