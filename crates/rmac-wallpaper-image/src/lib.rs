//! Bounded wallpaper decoding, procedural rasterization, and shared LRU cache.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher as _};
use std::io::BufReader;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

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
    detail: String,
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

    fn byte_len(&self) -> usize {
        self.rgba.len()
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum Key {
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

struct Entry {
    image: Arc<Decoded>,
    last_used: u64,
}

#[derive(Default)]
struct State {
    entries: HashMap<Key, Entry>,
    sequence: u64,
    bytes: usize,
    decodes: u64,
}

pub struct Cache {
    state: Mutex<State>,
    byte_budget: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RasterIssueKind {
    Resolve(rmac_wallpaper_system::ErrorKind),
    Decode(ErrorKind),
    Layout(rmac_wallpaper::LayoutError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RasterIssue {
    pub output: rmac_compositor::OutputId,
    pub kind: RasterIssueKind,
}

#[derive(Clone, Debug)]
pub struct RasterSurface {
    pub output: rmac_compositor::OutputId,
    pub logical_size: rmac_compositor::LogicalSize,
    pub scale: f64,
    pub fit: rmac_shell_settings::WallpaperFit,
    pub layout: rmac_wallpaper::Layout,
    pub image: Arc<Decoded>,
}

#[derive(Debug, Default)]
pub struct Rasterized {
    pub surfaces: Vec<RasterSurface>,
    pub issues: Vec<RasterIssue>,
}

/// Resolve, decode, and lay out every output independently. Any custom-file or
/// codec failure substitutes the original built-in only on that output.
pub fn rasterize(plan: &rmac_wallpaper::Plan, cache: &Cache) -> Rasterized {
    let mut rasterized = Rasterized::default();
    for surface in &plan.surfaces {
        let target = physical_target(surface.logical_size, surface.scale);
        let resolved = match rmac_wallpaper_system::resolve(&surface.source) {
            Ok(resolved) => resolved,
            Err(error) => {
                rasterized.issues.push(RasterIssue {
                    output: surface.output.clone(),
                    kind: RasterIssueKind::Resolve(error.kind),
                });
                rmac_wallpaper_system::ResolvedSource::BuiltIn(
                    rmac_wallpaper::DEFAULT_BUILT_IN.metadata(),
                )
            }
        };
        let image = match cache.get_or_decode(resolved, target) {
            Ok(image) => image,
            Err(error) => {
                rasterized.issues.push(RasterIssue {
                    output: surface.output.clone(),
                    kind: RasterIssueKind::Decode(error.kind),
                });
                match cache.get_or_decode(
                    rmac_wallpaper_system::ResolvedSource::BuiltIn(
                        rmac_wallpaper::DEFAULT_BUILT_IN.metadata(),
                    ),
                    target,
                ) {
                    Ok(image) => image,
                    Err(_) => continue,
                }
            }
        };
        let layout = match rmac_wallpaper::layout(
            surface.fit,
            image.physical_size(),
            surface.logical_size,
            surface.scale,
        ) {
            Ok(layout) => layout,
            Err(error) => {
                rasterized.issues.push(RasterIssue {
                    output: surface.output.clone(),
                    kind: RasterIssueKind::Layout(error),
                });
                continue;
            }
        };
        rasterized.surfaces.push(RasterSurface {
            output: surface.output.clone(),
            logical_size: surface.logical_size,
            scale: surface.scale,
            fit: surface.fit,
            layout,
            image,
        });
    }
    rasterized
}

fn physical_target(
    logical: rmac_compositor::LogicalSize,
    scale: f64,
) -> rmac_compositor::PhysicalSize {
    fn dimension(value: f64) -> u32 {
        if !value.is_finite() || value <= 0.0 {
            0
        } else if value >= f64::from(u32::MAX) {
            u32::MAX
        } else {
            value.round().max(1.0) as u32
        }
    }
    rmac_compositor::PhysicalSize {
        width: dimension(logical.width * scale),
        height: dimension(logical.height * scale),
    }
}

pub enum FileWatchEvent {
    Changed,
    Failed { detail: String },
}

impl fmt::Debug for FileWatchEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Changed => formatter.write_str("Changed"),
            Self::Failed { .. } => formatter
                .debug_struct("Failed")
                .field("detail", &"<redacted>")
                .finish(),
        }
    }
}

pub struct FileWatcher {
    _watcher: notify::RecommendedWatcher,
}

/// Watch exact selected files and their existing symlink targets. Parent
/// directories are non-recursive so unrelated tree activity is ignored.
pub fn watch_files(
    paths: &[std::path::PathBuf],
    sender: async_channel::Sender<FileWatchEvent>,
) -> Result<Option<FileWatcher>, Error> {
    use notify::Watcher as _;

    if paths.is_empty() {
        return Ok(None);
    }
    let mut targets = BTreeSet::new();
    for path in paths {
        targets.insert(path.clone());
        if let Ok(canonical) = path.canonicalize() {
            targets.insert(canonical);
        }
    }
    let callback_targets = targets.clone();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let event = match result {
            Ok(event)
                if !matches!(event.kind, notify::EventKind::Access(_))
                    && event.paths.iter().any(|path| {
                        callback_targets.contains(path)
                            || callback_targets
                                .iter()
                                .any(|target| target.starts_with(path))
                    }) =>
            {
                Some(FileWatchEvent::Changed)
            }
            Ok(_) => None,
            Err(error) => Some(FileWatchEvent::Failed {
                detail: error.to_string(),
            }),
        };
        if let Some(event) = event {
            let _ = sender.try_send(event);
        }
    })
    .map_err(|error| failure(ErrorKind::Watch, error.to_string()))?;
    let parents: BTreeSet<_> = targets
        .iter()
        .filter_map(|path| path.parent().map(Path::to_path_buf))
        .collect();
    for parent in parents {
        watcher
            .watch(&parent, notify::RecursiveMode::NonRecursive)
            .map_err(|error| failure(ErrorKind::Watch, error.to_string()))?;
    }
    Ok(Some(FileWatcher { _watcher: watcher }))
}

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

    /// Decode or rasterize on the caller's background worker. The lock keeps
    /// simultaneous requests for one source from duplicating large RGBA data.
    pub fn get_or_decode(
        &self,
        source: rmac_wallpaper_system::ResolvedSource,
        target: rmac_compositor::PhysicalSize,
    ) -> Result<Arc<Decoded>, Error> {
        let key = key(&source, target)?;
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
        let image = Arc::new(decode(source, target)?);
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
) -> Result<Key, Error> {
    match source {
        rmac_wallpaper_system::ResolvedSource::BuiltIn(metadata) => {
            validate_dimensions(target.width, target.height)?;
            Ok(Key::BuiltIn {
                id: metadata.id,
                width: target.width,
                height: target.height,
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
) -> Result<Decoded, Error> {
    match source {
        rmac_wallpaper_system::ResolvedSource::BuiltIn(metadata) => procedural(metadata, target),
        rmac_wallpaper_system::ResolvedSource::File(asset) => decode_file(asset),
    }
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
) -> Result<Decoded, Error> {
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
            let from = rgb(metadata.palette[index]);
            let to = rgb(metadata.palette[index + 1]);
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

fn failure(kind: ErrorKind, detail: impl Into<String>) -> Error {
    Error {
        kind,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn target(width: u32, height: u32) -> rmac_compositor::PhysicalSize {
        rmac_compositor::PhysicalSize { width, height }
    }

    #[test]
    fn procedural_default_is_deterministic_bounded_and_cached_by_target() {
        let cache = Cache::new(1024 * 1024);
        let source = || {
            rmac_wallpaper_system::ResolvedSource::BuiltIn(
                rmac_wallpaper::BuiltInId::Aurora.metadata(),
            )
        };
        let first = cache.get_or_decode(source(), target(64, 32)).unwrap();
        let second = cache.get_or_decode(source(), target(64, 32)).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.rgba.len(), 64 * 32 * 4);
        assert_eq!(cache.decode_count(), 1);
        assert!(!Arc::ptr_eq(
            &first,
            &cache.get_or_decode(source(), target(32, 32)).unwrap()
        ));
        assert_eq!(cache.decode_count(), 2);
    }

    #[test]
    fn user_file_decode_is_shared_across_outputs_and_explicitly_invalidated() {
        let root = temporary_directory("file-cache");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("wallpaper.png");
        image::RgbaImage::from_pixel(8, 4, image::Rgba([10, 20, 30, 255]))
            .save(&path)
            .unwrap();
        let cache = Cache::default();
        let source =
            || rmac_wallpaper_system::resolve(&rmac_wallpaper::Source::File(path.clone())).unwrap();
        let first = cache.get_or_decode(source(), target(1920, 1080)).unwrap();
        let second = cache.get_or_decode(source(), target(3840, 2160)).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.physical_size(), target(8, 4));
        assert_eq!(cache.decode_count(), 1);
        assert_eq!(cache.invalidate_path(&path.canonicalize().unwrap()), 1);
        let third = cache.get_or_decode(source(), target(1920, 1080)).unwrap();
        assert!(!Arc::ptr_eq(&first, &third));
        assert_eq!(cache.decode_count(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bounds_reject_zero_per_axis_and_decompression_sized_targets() {
        let cache = Cache::default();
        let source = || {
            rmac_wallpaper_system::ResolvedSource::BuiltIn(
                rmac_wallpaper::BuiltInId::Aurora.metadata(),
            )
        };
        assert_eq!(
            cache
                .get_or_decode(source(), target(0, 10))
                .unwrap_err()
                .kind,
            ErrorKind::InvalidDimensions
        );
        assert_eq!(
            cache
                .get_or_decode(source(), target(10_000, 10_000))
                .unwrap_err()
                .kind,
            ErrorKind::TooManyPixels
        );
    }

    #[test]
    fn lru_budget_evicts_old_images_without_invalidating_live_arcs() {
        let cache = Cache::new(4 * 4 * 4);
        let source = || {
            rmac_wallpaper_system::ResolvedSource::BuiltIn(
                rmac_wallpaper::BuiltInId::Aurora.metadata(),
            )
        };
        let first = cache.get_or_decode(source(), target(4, 4)).unwrap();
        cache.get_or_decode(source(), target(2, 2)).unwrap();
        assert_eq!(cache.len(), 1);
        assert_eq!(first.rgba.len(), 64);
        assert_eq!(first.rgba[3], 255);
    }

    #[test]
    fn empty_watch_set_needs_no_native_watcher_and_failures_are_redacted() {
        let (sender, _) = async_channel::bounded(1);
        assert!(watch_files(&[], sender).unwrap().is_none());
        let event = FileWatchEvent::Failed {
            detail: "/home/alex/private/wallpaper.png".into(),
        };
        assert!(!format!("{event:?}").contains("alex"));
    }

    #[test]
    fn rasterization_shares_user_decode_across_outputs_and_falls_back_per_failure() {
        let root = temporary_directory("rasterize");
        std::fs::create_dir_all(&root).unwrap();
        let valid = root.join("valid.png");
        image::RgbaImage::from_pixel(8, 4, image::Rgba([1, 2, 3, 255]))
            .save(&valid)
            .unwrap();
        let corrupt = root.join("corrupt.png");
        std::fs::write(&corrupt, b"\x89PNG\r\n\x1a\ncorrupt").unwrap();
        let surface =
            |output: &str, source: std::path::PathBuf, scale: f64| rmac_wallpaper::Surface {
                output: output.into(),
                logical_size: rmac_compositor::LogicalSize {
                    width: 16.0,
                    height: 9.0,
                },
                scale,
                fit: rmac_shell_settings::WallpaperFit::Fill,
                source: rmac_wallpaper::Source::File(source),
            };
        let plan = rmac_wallpaper::Plan {
            surfaces: vec![
                surface("DP-1", valid.clone(), 1.0),
                surface("DP-2", valid, 2.0),
                surface("DP-3", corrupt, 1.0),
                surface("DP-4", root.join("missing.png"), 1.0),
            ],
            issues: Vec::new(),
        };
        let cache = Cache::default();
        let rasterized = rasterize(&plan, &cache);
        assert_eq!(rasterized.surfaces.len(), 4);
        assert!(Arc::ptr_eq(
            &rasterized.surfaces[0].image,
            &rasterized.surfaces[1].image
        ));
        assert_eq!(rasterized.issues.len(), 2);
        assert!(rasterized
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, RasterIssueKind::Decode(_))));
        assert!(rasterized
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, RasterIssueKind::Resolve(_))));
        assert_eq!(rasterized.surfaces[2].image.physical_size(), target(16, 9));
        assert_eq!(rasterized.surfaces[3].image.physical_size(), target(16, 9));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn temporary_directory(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-wallpaper-image-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
}
