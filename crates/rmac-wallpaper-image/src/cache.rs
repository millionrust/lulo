use std::hash::{DefaultHasher, Hash, Hasher as _};
use std::io::BufReader;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use crate::model::{Cache, Decoded, Entry, Error, ErrorKind, Key, State};
use crate::{DEFAULT_CACHE_BYTES, MAX_DIMENSION, MAX_PIXELS};

impl Default for Cache {
    fn default() -> Self {
        Self::new(DEFAULT_CACHE_BYTES)
    }
}

impl Cache {
    pub fn new(byte_budget: usize) -> Self {
        Self {
            state: Mutex::new(State::default()),
            byte_budget,
        }
    }

    /// Decode or rasterize for the dark appearance. See [`Self::get_or_decode_for`].
    pub fn get_or_decode(
        &self,
        source: rmac_wallpaper_system::ResolvedSource,
        target: rmac_compositor::PhysicalSize,
    ) -> Result<Arc<Decoded>, Error> {
        self.get_or_decode_for(source, target, true)
    }

    /// Decode or rasterize on the caller's background worker. Built-ins use
    /// their light or dark form for `dark`; user files ignore it. The lock
    /// keeps simultaneous requests for one source from duplicating RGBA data.
    pub fn get_or_decode_for(
        &self,
        source: rmac_wallpaper_system::ResolvedSource,
        target: rmac_compositor::PhysicalSize,
        dark: bool,
    ) -> Result<Arc<Decoded>, Error> {
        let key = key(&source, target, dark)?;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.sequence = state.sequence.wrapping_add(1).max(1);
        let sequence = state.sequence;
        if let Some(entry) = state.entries.get_mut(&key) {
            entry.last_used = sequence;
            return Ok(entry.image.clone());
        }
        let image = Arc::new(decode(source, target, dark)?);
        state.decodes = state.decodes.saturating_add(1);
        let bytes = image.byte_len();
        if bytes <= self.byte_budget {
            while state.bytes.saturating_add(bytes) > self.byte_budget {
                let Some(evict) = state
                    .entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.last_used)
                    .map(|(key, _)| key.clone())
                else {
                    break;
                };
                if let Some(removed) = state.entries.remove(&evict) {
                    state.bytes = state.bytes.saturating_sub(removed.image.byte_len());
                }
            }
            state.bytes = state.bytes.saturating_add(bytes);
            state.entries.insert(
                key,
                Entry {
                    image: image.clone(),
                    last_used: sequence,
                },
            );
        }
        Ok(image)
    }

    /// Remove every cached fingerprint for a changed path. Call from a native
    /// file-watch event; this cache performs no polling of its own.
    pub fn invalidate_path(&self, path: &Path) -> usize {
        let identity = path_identity(path);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let keys: Vec<_> = state
            .entries
            .keys()
            .filter(|key| matches!(key, Key::File { path_hash, .. } if *path_hash == identity))
            .cloned()
            .collect();
        for key in &keys {
            if let Some(removed) = state.entries.remove(key) {
                state.bytes = state.bytes.saturating_sub(removed.image.byte_len());
            }
        }
        keys.len()
    }

    pub fn len(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entries
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn decode_count(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .decodes
    }
}

fn key(
    source: &rmac_wallpaper_system::ResolvedSource,
    target: rmac_compositor::PhysicalSize,
    dark: bool,
) -> Result<Key, Error> {
    match source {
        rmac_wallpaper_system::ResolvedSource::BuiltIn(metadata) => {
            validate_dimensions(target.width, target.height)?;
            // Artwork is one packaged image per size tier, so every output in
            // that tier shares a single decode.
            let (width, height) = if metadata.has_artwork {
                rmac_wallpaper::artwork_size(target.width, target.height)
            } else {
                (target.width, target.height)
            };
            Ok(Key::BuiltIn {
                id: metadata.id,
                width,
                height,
                dark,
            })
        }
        rmac_wallpaper_system::ResolvedSource::File(asset) => Ok(Key::File {
            path_hash: path_identity(asset.canonical_path()),
            byte_len: asset.byte_len,
            modified_nanos: asset
                .modified
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or_default(),
            format: asset.format,
        }),
    }
}

fn decode(
    source: rmac_wallpaper_system::ResolvedSource,
    target: rmac_compositor::PhysicalSize,
    dark: bool,
) -> Result<Decoded, Error> {
    match source {
        rmac_wallpaper_system::ResolvedSource::BuiltIn(metadata) if metadata.has_artwork => {
            artwork(metadata, target, dark)
        }
        rmac_wallpaper_system::ResolvedSource::BuiltIn(metadata) => {
            procedural(metadata, target, dark)
        }
        rmac_wallpaper_system::ResolvedSource::File(asset) => decode_file(asset),
    }
}

/// Open the packaged image through the same bounded file authority and
/// decoder as user files. A missing or unreadable image is an error, never a
/// silent substitute: the caller records it and shows the fallback.
fn artwork(
    metadata: rmac_wallpaper::BuiltInMetadata,
    target: rmac_compositor::PhysicalSize,
    dark: bool,
) -> Result<Decoded, Error> {
    validate_dimensions(target.width, target.height)?;
    let path = metadata
        .artwork_path(dark, target.width, target.height)
        .ok_or_else(|| failure(ErrorKind::Artwork, "built-in has no packaged artwork"))?;
    let asset = rmac_wallpaper_system::open_file(&path).map_err(|error| {
        failure(
            ErrorKind::Artwork,
            format!(
                "the packaged {} wallpaper image is unavailable: {}",
                metadata.title,
                error.detail()
            ),
        )
    })?;
    decode_file(asset)
}

fn decode_file(asset: rmac_wallpaper_system::FileAsset) -> Result<Decoded, Error> {
    let format = match asset.format {
        rmac_wallpaper_system::ImageFormat::Png => image::ImageFormat::Png,
        rmac_wallpaper_system::ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        rmac_wallpaper_system::ImageFormat::WebP => image::ImageFormat::WebP,
    };
    let mut reader = image::ImageReader::with_format(BufReader::new(asset.into_file()), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_PIXELS * 4);
    reader.limits(limits);
    let image = reader.decode().map_err(|error| Error {
        kind: ErrorKind::Decode,
        detail: error.to_string(),
    })?;
    validate_dimensions(image.width(), image.height())?;
    let image = image.into_rgba8();
    Ok(Decoded {
        width: image.width(),
        height: image.height(),
        rgba: Arc::from(image.into_raw()),
    })
}

fn procedural(
    metadata: rmac_wallpaper::BuiltInMetadata,
    target: rmac_compositor::PhysicalSize,
    dark: bool,
) -> Result<Decoded, Error> {
    let palette = metadata.palette_for(dark);
    validate_dimensions(target.width, target.height)?;
    let capacity = usize::try_from(u64::from(target.width) * u64::from(target.height) * 4)
        .map_err(|_| {
            failure(
                ErrorKind::TooManyPixels,
                "pixel buffer exceeds address space",
            )
        })?;
    let mut rgba = Vec::with_capacity(capacity);
    for y in 0..target.height {
        let vertical = if target.height > 1 {
            y as f32 / (target.height - 1) as f32
        } else {
            0.0
        };
        for x in 0..target.width {
            let horizontal = if target.width > 1 {
                x as f32 / (target.width - 1) as f32
            } else {
                0.0
            };
            let position = (horizontal * 0.58 + vertical * 0.42).clamp(0.0, 1.0);
            let scaled = position * 3.0;
            let index = (scaled.floor() as usize).min(2);
            let amount = scaled - index as f32;
            let from = rgb(palette[index]);
            let to = rgb(palette[index + 1]);
            rgba.extend_from_slice(&[
                lerp(from[0], to[0], amount),
                lerp(from[1], to[1], amount),
                lerp(from[2], to[2], amount),
                255,
            ]);
        }
    }
    Ok(Decoded {
        width: target.width,
        height: target.height,
        rgba: Arc::from(rgba),
    })
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), Error> {
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(failure(
            ErrorKind::InvalidDimensions,
            "image dimensions are zero or exceed the per-axis limit",
        ));
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(failure(
            ErrorKind::TooManyPixels,
            "image exceeds the decoded-pixel limit",
        ));
    }
    Ok(())
}

fn path_identity(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.canonicalize()
        .as_deref()
        .unwrap_or(path)
        .hash(&mut hasher);
    hasher.finish()
}

fn rgb(value: u32) -> [u8; 3] {
    [
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    ]
}

fn lerp(from: u8, to: u8, amount: f32) -> u8 {
    (f32::from(from) + (f32::from(to) - f32::from(from)) * amount).round() as u8
}

pub(crate) fn failure(kind: ErrorKind, detail: impl Into<String>) -> Error {
    Error {
        kind,
        detail: detail.into(),
    }
}
