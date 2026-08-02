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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Key {
    BuiltIn {
        id: rmac_wallpaper::BuiltInId,
        width: u32,
        height: u32,
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
