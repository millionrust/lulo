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

/// Inter tracking per role, measured against SF (FEEL_SPEC.md §D.5). Values are
/// interpolated so an unlisted base size still lands on the curve.
fn tracking(base_size: f32) -> f32 {
    const TABLE: [(f32, f32); 9] = [
        (10.0, 0.12),
        (11.0, 0.10),
        (12.0, 0.06),
        (13.0, 0.02),
        (15.0, 0.0),
        (17.0, -0.10),
        (22.0, -0.25),
        (26.0, -0.35),
        (96.0, -1.5),
    ];
    if base_size <= TABLE[0].0 {
        return TABLE[0].1;
    }
    for window in TABLE.windows(2) {
        let (low_size, low_tracking) = window[0];
        let (high_size, high_tracking) = window[1];
        if base_size <= high_size {
            let t = (base_size - low_size) / (high_size - low_size);
            return low_tracking + (high_tracking - low_tracking) * t;
        }
    }
    TABLE[TABLE.len() - 1].1
}

impl TextStyle {
    fn new(base_size: f32, weight: Weight, factor: f32) -> Self {
        let size = base_size * factor;
        Self {
            size,
            weight,
            line_height: (size * 1.23).round(),
            letter_spacing: tracking(base_size),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_follows_the_measured_sf_curve() {
        assert_eq!(tracking(10.0), 0.12);
        assert_eq!(tracking(13.0), 0.02);
        assert_eq!(tracking(15.0), 0.0);
        assert_eq!(tracking(26.0), -0.35);
        assert_eq!(tracking(96.0), -1.5);
        // A size between table rows interpolates instead of snapping.
        let twenty = tracking(20.0);
        assert!(twenty < -0.10 && twenty > -0.25, "20px tracking was {twenty}");
    }
}
