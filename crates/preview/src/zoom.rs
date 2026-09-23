//! Zoom arithmetic: fit scales, the measured zoom steps, and keeping the
//! point under the viewport centre still while the scale changes.

/// Smallest and largest scales rmac allows. Preview's own limits were not
/// measured; these keep a Letter page between ~61 pt and ~9800 pt wide.
pub const MIN_SCALE: f32 = 0.1;
pub const MAX_SCALE: f32 = 16.0;

/// PDF zoom in/out multiplies or divides by 2^(1/4): measured on macOS 26
/// Preview as page widths 1092 → 1298 → 1544 → 1836 pt.
pub const PDF_STEP: f32 = 1.189_207_1;

/// Image zoom steps. The part at or below 1 is measured on macOS 26 Preview
/// (⌘− from actual size: 1 → .75 → .5 → .4 → .3 → .2); 0.1 and everything
/// above 1 extend the table and were not measured.
pub const IMAGE_STEPS: [f32; 13] = [
    0.1, 0.2, 0.3, 0.4, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0,
];

/// Horizontal room Preview leaves beside a width-fitted PDF page: a 928 pt
/// page in a 953 pt document column.
pub const PDF_FIT_MARGIN: f32 = 25.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentKind {
    Image,
    Pdf,
}

/// The zoom state of one document. `Fit` follows the viewport as the window
/// resizes; `Scale` is a fixed factor (1 = actual size).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    Fit,
    Scale(f32),
}

impl Zoom {
    /// The concrete scale for this zoom given the current fit scale.
    pub fn resolve(self, fit: f32) -> f32 {
        match self {
            Zoom::Fit => fit,
            Zoom::Scale(scale) => scale,
        }
    }
}

/// Scale that shows the whole image inside the viewport ("Zoom to Fit").
/// Preview enlarges small images to fill the window as well.
pub fn fit_contain(content: (f32, f32), viewport: (f32, f32)) -> f32 {
    let (width, height) = content;
    let (view_width, view_height) = viewport;
    if width <= 0.0 || height <= 0.0 || view_width <= 0.0 || view_height <= 0.0 {
        return 1.0;
    }
    clamp((view_width / width).min(view_height / height))
}

/// Scale that makes the widest page fill the document column less the
/// measured margin (PDF "Zoom to Fit" in continuous scroll).
pub fn fit_width(widest_page: f32, viewport_width: f32) -> f32 {
    if widest_page <= 0.0 {
        return 1.0;
    }
    clamp(((viewport_width - PDF_FIT_MARGIN).max(1.0)) / widest_page)
}

pub fn clamp(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(MIN_SCALE, MAX_SCALE)
    } else {
        1.0
    }
}

/// The next larger scale for ⌘+.
pub fn zoom_in(kind: ContentKind, scale: f32) -> f32 {
    match kind {
        ContentKind::Pdf => clamp(scale * PDF_STEP),
        ContentKind::Image => IMAGE_STEPS
            .iter()
            .copied()
            .find(|step| *step > scale * 1.001)
            .unwrap_or(MAX_SCALE.min(IMAGE_STEPS[IMAGE_STEPS.len() - 1])),
    }
}

/// The next smaller scale for ⌘−.
pub fn zoom_out(kind: ContentKind, scale: f32) -> f32 {
    match kind {
        ContentKind::Pdf => clamp(scale / PDF_STEP),
        ContentKind::Image => IMAGE_STEPS
            .iter()
            .rev()
            .copied()
            .find(|step| *step < scale * 0.999)
            .unwrap_or(IMAGE_STEPS[0]),
    }
}

/// Scroll offset (distance scrolled, ≥ 0) that keeps the content point under
/// the viewport centre fixed when the scale changes from `old` to `new`.
pub fn anchored_scroll(scrolled: f32, viewport: f32, old: f32, new: f32) -> f32 {
    if old <= 0.0 {
        return 0.0;
    }
    let centre = scrolled + viewport / 2.0;
    (centre * new / old - viewport / 2.0).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contain_fits_the_measured_multi_image_window() {
        // 800×600 image beside the sidebar: a 632×600 column showed 631×474.
        let scale = fit_contain((800.0, 600.0), (632.0, 600.0));
        assert!((800.0 * scale - 632.0).abs() < 0.01);
        assert!((600.0 * scale - 474.0).abs() < 0.5);
    }

    #[test]
    fn contain_enlarges_small_images_and_rejects_empty_input() {
        assert_eq!(fit_contain((100.0, 100.0), (400.0, 300.0)), 3.0);
        assert_eq!(fit_contain((0.0, 100.0), (400.0, 300.0)), 1.0);
        assert_eq!(fit_contain((100.0, 100.0), (0.0, 300.0)), 1.0);
    }

    #[test]
    fn fit_width_matches_the_measured_letter_page() {
        // 953 pt column → 928 pt Letter page (612 pt wide).
        let scale = fit_width(612.0, 953.0);
        assert!((612.0 * scale - 928.0).abs() < 0.01);
    }

    #[test]
    fn pdf_steps_follow_the_measured_fourth_root_of_two() {
        let mut width = 1092.0_f32 / 612.0;
        let expected = [1298.0, 1544.0, 1836.0];
        for target in expected {
            width = zoom_in(ContentKind::Pdf, width);
            assert!((width * 612.0 - target).abs() < 2.0, "{}", width * 612.0);
        }
        let back = zoom_out(ContentKind::Pdf, zoom_in(ContentKind::Pdf, 1.0));
        assert!((back - 1.0).abs() < 1e-5);
    }

    #[test]
    fn image_zoom_out_walks_the_measured_steps() {
        let mut scale = 1.0;
        let mut seen = Vec::new();
        for _ in 0..5 {
            scale = zoom_out(ContentKind::Image, scale);
            seen.push(scale);
        }
        assert_eq!(seen, [0.75, 0.5, 0.4, 0.3, 0.2]);
    }

    #[test]
    fn image_zoom_snaps_from_a_fit_scale_to_the_next_step() {
        assert_eq!(zoom_in(ContentKind::Image, 0.79), 1.0);
        assert_eq!(zoom_out(ContentKind::Image, 0.79), 0.75);
        assert_eq!(zoom_in(ContentKind::Image, 8.0), 8.0);
        assert_eq!(zoom_out(ContentKind::Image, 0.1), 0.1);
    }

    #[test]
    fn scales_are_clamped() {
        assert_eq!(zoom_in(ContentKind::Pdf, MAX_SCALE), MAX_SCALE);
        assert_eq!(zoom_out(ContentKind::Pdf, MIN_SCALE), MIN_SCALE);
        assert_eq!(clamp(f32::NAN), 1.0);
    }

    #[test]
    fn zoom_resolves_fit_against_the_current_viewport() {
        assert_eq!(Zoom::Fit.resolve(0.8), 0.8);
        assert_eq!(Zoom::Scale(2.0).resolve(0.8), 2.0);
    }

    #[test]
    fn anchored_scroll_keeps_the_centre_point() {
        // Centre at 500 of content; doubling puts it at 1000.
        assert_eq!(anchored_scroll(300.0, 400.0, 1.0, 2.0), 800.0);
        assert_eq!(anchored_scroll(0.0, 400.0, 2.0, 1.0), 0.0);
        assert_eq!(anchored_scroll(10.0, 400.0, 0.0, 1.0), 0.0);
    }
}
