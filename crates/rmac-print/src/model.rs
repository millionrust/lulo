use std::fmt;

pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;
pub const MAX_PAGES: usize = 256;
pub(crate) const MAX_RASTER_BYTES: usize = 96 * 1024 * 1024;
pub(crate) const MAX_PDF_BYTES: usize = 128 * 1024 * 1024;
pub(crate) const POINTS_PER_INCH: f64 = 72.0;
pub(crate) const MILLIMETERS_PER_INCH: f64 = 25.4;

use crate::render::validate_layout;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageLayout {
    pub width_mm: f64,
    pub height_mm: f64,
    pub margin_top_mm: f64,
    pub margin_right_mm: f64,
    pub margin_bottom_mm: f64,
    pub margin_left_mm: f64,
    pub dpi: u16,
}

impl Default for PageLayout {
    fn default() -> Self {
        Self {
            width_mm: 210.0,
            height_mm: 297.0,
            margin_top_mm: 15.0,
            margin_right_mm: 15.0,
            margin_bottom_mm: 15.0,
            margin_left_mm: 15.0,
            dpi: 144,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageOrientation {
    Portrait,
    Landscape,
}

impl PageOrientation {
    /// Parse both spellings used by the portal's settings and page-setup
    /// dictionaries. Reverse orientation changes physical feed direction, not
    /// the portrait/landscape geometry required by this renderer.
    pub fn from_portal(value: &str) -> Option<Self> {
        match value {
            "portrait" | "reverse_portrait" | "reverse-portrait" => Some(Self::Portrait),
            "landscape" | "reverse_landscape" | "reverse-landscape" => Some(Self::Landscape),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PageDescription {
    pub width_mm: Option<f64>,
    pub height_mm: Option<f64>,
    pub margin_top_mm: Option<f64>,
    pub margin_right_mm: Option<f64>,
    pub margin_bottom_mm: Option<f64>,
    pub margin_left_mm: Option<f64>,
    pub orientation: Option<PageOrientation>,
}

impl PageLayout {
    /// Convert a portal page description into the renderer's exact layout.
    /// Missing values use the documented A4 defaults; invalid values remain
    /// errors rather than being silently clamped.
    pub fn from_description(description: PageDescription) -> Result<Self, Error> {
        let defaults = Self::default();
        let mut width_mm = description.width_mm.unwrap_or(defaults.width_mm);
        let mut height_mm = description.height_mm.unwrap_or(defaults.height_mm);
        match description.orientation {
            Some(PageOrientation::Landscape) if width_mm < height_mm => {
                std::mem::swap(&mut width_mm, &mut height_mm);
            }
            Some(PageOrientation::Portrait) if width_mm > height_mm => {
                std::mem::swap(&mut width_mm, &mut height_mm);
            }
            _ => {}
        }
        let layout = Self {
            width_mm,
            height_mm,
            margin_top_mm: description.margin_top_mm.unwrap_or(defaults.margin_top_mm),
            margin_right_mm: description
                .margin_right_mm
                .unwrap_or(defaults.margin_right_mm),
            margin_bottom_mm: description
                .margin_bottom_mm
                .unwrap_or(defaults.margin_bottom_mm),
            margin_left_mm: description
                .margin_left_mm
                .unwrap_or(defaults.margin_left_mm),
            dpi: defaults.dpi,
        };
        validate_layout(layout).map(|_| layout)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    SourceTooLarge,
    InvalidPageLayout,
    TooManyPages,
    RasterLimit,
    FontUnavailable,
    Encode,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SourceTooLarge => "the document exceeds the 1 MiB print safety limit",
            Self::InvalidPageLayout => "the selected page layout is not supported",
            Self::TooManyPages => "the document exceeds the 256-page print safety limit",
            Self::RasterLimit => "the printable document exceeds the rendering memory limit",
            Self::FontUnavailable => "no system font could render the document",
            Self::Encode => "the printable PDF could not be encoded safely",
        })
    }
}

impl std::error::Error for Error {}

pub(crate) struct ValidLayout {
    pub(crate) width_px: usize,
    pub(crate) height_px: usize,
    pub(crate) margin_top_px: usize,
    pub(crate) margin_left_px: usize,
    pub(crate) content_width_px: usize,
    pub(crate) content_height_px: usize,
    pub(crate) line_height_px: usize,
    pub(crate) font_size_px: f32,
    pub(crate) width_points: f64,
    pub(crate) height_points: f64,
}
