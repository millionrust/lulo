//! Typography roles scaled by the user's `TextScale`.

use rmac_appearance::TextScale;

/// Font weight as plain data (GPUI-free).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Medium,
    Semibold,
    Bold,
}

/// One resolved text role.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub weight: Weight,
    pub line_height: f32,
    pub letter_spacing: f32,
}

impl TextStyle {
    fn new(base_size: f32, weight: Weight, factor: f32) -> Self {
        let size = base_size * factor;
        let letter_spacing = if base_size <= 13.0 {
            0.1
        } else if base_size >= 20.0 {
            -0.2
        } else {
            0.0
        };
        Self {
            size,
            weight,
            line_height: (size * 1.23).round(),
            letter_spacing,
        }
    }
}

/// Every typographic role in the product.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeScale {
    pub large_title: TextStyle,
    pub title1: TextStyle,
    pub title2: TextStyle,
    pub title3: TextStyle,
    pub headline: TextStyle,
    pub body: TextStyle,
    pub callout: TextStyle,
    pub subheadline: TextStyle,
    pub footnote: TextStyle,
    pub caption: TextStyle,
    pub mono_body: TextStyle,
    pub clock_menubar: TextStyle,
    pub lock_time: TextStyle,
    pub lock_date: TextStyle,
}

impl TypeScale {
    pub fn resolve(text_scale: TextScale) -> Self {
        let factor = text_scale.factor();
        Self {
            large_title: TextStyle::new(26.0, Weight::Regular, factor),
            title1: TextStyle::new(22.0, Weight::Regular, factor),
            title2: TextStyle::new(17.0, Weight::Semibold, factor),
            title3: TextStyle::new(15.0, Weight::Semibold, factor),
            headline: TextStyle::new(13.0, Weight::Semibold, factor),
            body: TextStyle::new(13.0, Weight::Regular, factor),
            callout: TextStyle::new(12.0, Weight::Regular, factor),
            subheadline: TextStyle::new(11.0, Weight::Regular, factor),
            footnote: TextStyle::new(10.0, Weight::Regular, factor),
            caption: TextStyle::new(10.0, Weight::Medium, factor),
            mono_body: TextStyle::new(12.0, Weight::Regular, factor),
            clock_menubar: TextStyle::new(13.0, Weight::Medium, factor),
            lock_time: TextStyle::new(96.0, Weight::Semibold, factor),
            lock_date: TextStyle::new(20.0, Weight::Medium, factor),
        }
    }
}
