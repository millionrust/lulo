use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

pub const MAX_DIMENSION: u32 = 16_384;
pub const MAX_PIXELS: u64 = 40_000_000;
pub const DEFAULT_CACHE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    InvalidDimensions,
    TooManyPixels,
    Decode,
    Watch,
    /// A packaged built-in image is missing or unreadable.
    Artwork,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Error {
    pub kind: ErrorKind,
    pub(crate) detail: String,
}

impl Error {
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Error")
            .field("kind", &self.kind)
            .field("detail", &"<redacted>")
            .finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Could not decode the wallpaper image")
    }
}

impl std::error::Error for Error {}

pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

impl fmt::Debug for Decoded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Decoded")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.rgba.len())
            .finish()
    }
}

impl Decoded {
    pub fn physical_size(&self) -> rmac_compositor::PhysicalSize {
        rmac_compositor::PhysicalSize {
            width: self.width,
            height: self.height,
        }
    }

    pub(crate) fn byte_len(&self) -> usize {
        self.rgba.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorSummary {
    pub dominant: [u8; 3],
    pub luminance: f32,
}

/// Average an 8×8 nearest-neighbour downsample in sRGB, as required by the
/// shared wallpaper colour authority. This is bounded to 64 pixel reads even
/// for the maximum accepted wallpaper dimensions.
pub fn summarize_color(image: &Decoded) -> Option<ColorSummary> {
    if image.width == 0 || image.height == 0 {
        return None;
    }
    let expected = u64::from(image.width)
        .checked_mul(u64::from(image.height))?
        .checked_mul(4)?;
    if expected != image.rgba.len() as u64 {
        return None;
    }

    let mut sums = [0_u32; 3];
    for row in 0_u64..8 {
        let y = (((row * 2 + 1) * u64::from(image.height)) / 16).min(u64::from(image.height - 1));
        for column in 0_u64..8 {
            let x =
                (((column * 2 + 1) * u64::from(image.width)) / 16).min(u64::from(image.width - 1));
            let offset = ((y * u64::from(image.width) + x) * 4) as usize;
            sums[0] += u32::from(image.rgba[offset]);
            sums[1] += u32::from(image.rgba[offset + 1]);
            sums[2] += u32::from(image.rgba[offset + 2]);
        }
    }
    let dominant = sums.map(|sum| ((sum + 32) / 64) as u8);
    let linear = |value: u8| {
        let value = f32::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance =
        0.2126 * linear(dominant[0]) + 0.7152 * linear(dominant[1]) + 0.0722 * linear(dominant[2]);
    Some(ColorSummary {
        dominant,
        luminance,
    })
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Key {
    BuiltIn {
        id: rmac_wallpaper::BuiltInId,
        width: u32,
        height: u32,
        dark: bool,
    },
    File {
        path_hash: u64,
        byte_len: u64,
        modified_nanos: u128,
        format: rmac_wallpaper_system::ImageFormat,
    },
}

pub(crate) struct Entry {
    pub(crate) image: Arc<Decoded>,
    pub(crate) last_used: u64,
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) entries: HashMap<Key, Entry>,
    pub(crate) sequence: u64,
    pub(crate) bytes: usize,
    pub(crate) decodes: u64,
}

pub struct Cache {
    pub(crate) state: Mutex<State>,
    pub(crate) byte_budget: usize,
}
