use std::path::{Component, Path, PathBuf};

/// What a new user sees: the Lulo hero wallpaper.
pub const DEFAULT_BUILT_IN: BuiltInId = BuiltInId::Lulo;
/// What replaces a source that cannot be shown. Aurora is drawn by the
/// renderer with no file access, so this fallback cannot itself fail.
pub const FALLBACK_BUILT_IN: BuiltInId = BuiltInId::Aurora;

/// Where the session package installs the Lulo wallpaper images
/// (packaging/rmac-session/wallpapers, scripts/build-wallpapers.py).
pub const PACKAGED_WALLPAPER_DIR: &str = "/usr/share/rmac/wallpapers";
/// Overrides [`PACKAGED_WALLPAPER_DIR`] for development builds that run
/// from a source checkout instead of the installed package.
pub const PACKAGED_WALLPAPER_DIR_ENV: &str = "RMAC_WALLPAPER_DIR";
/// Packaged artwork sizes, smallest first. The renderer picks the smallest
/// one that covers the output so a 1080p panel never decodes a 4K image.
pub const ARTWORK_SIZES: [(u32, u32); 3] = [(1920, 1080), (2560, 1600), (3840, 2160)];

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BuiltInId {
    Lulo,
    LuloGrove,
    LuloEmber,
    LuloDusk,
    LuloMist,
    LuloNocturne,
    Aurora,
    Tide,
    Basalt,
    Monsoon,
    Paper,
}

impl BuiltInId {
    /// Every original built-in, in picker order. `Lulo` is the default; the
    /// procedural set that preceded it stays available after it.
    pub const ALL: [BuiltInId; 11] = [
        BuiltInId::Lulo,
        BuiltInId::LuloGrove,
        BuiltInId::LuloEmber,
        BuiltInId::LuloDusk,
        BuiltInId::LuloMist,
        BuiltInId::LuloNocturne,
        BuiltInId::Aurora,
        BuiltInId::Tide,
        BuiltInId::Basalt,
        BuiltInId::Monsoon,
        BuiltInId::Paper,
    ];

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.id() == value)
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Lulo => "lulo",
            Self::LuloGrove => "lulo-grove",
            Self::LuloEmber => "lulo-ember",
            Self::LuloDusk => "lulo-dusk",
            Self::LuloMist => "lulo-mist",
            Self::LuloNocturne => "lulo-nocturne",
            Self::Aurora => "rmac-aurora",
            Self::Tide => "rmac-tide",
            Self::Basalt => "rmac-basalt",
            Self::Monsoon => "rmac-monsoon",
            Self::Paper => "rmac-paper",
        }
    }

    pub fn metadata(self) -> BuiltInMetadata {
        // Original sRGB four-stop palettes. Procedural built-ins are drawn from
        // them as a diagonal gradient; the Lulo set ships original rendered
        // artwork (scripts/build-wallpapers.py) and keeps a palette of its own
        // colours for the gallery swatch shown if that artwork is missing. No
        // third-party bitmap is ever bundled (FEEL_SPEC.md §D.6).
        let (title, palette, light_palette, has_artwork) = match self {
            Self::Lulo => (
                "Lulo",
                [0x0a1712, 0x2f7d4a, 0x86c846, 0xf08a24],
                [0xf5eee3, 0xc4e57e, 0x98cc58, 0xf59a3c],
                true,
            ),
            Self::LuloGrove => (
                "Grove",
                [0x07110d, 0x1c5a3e, 0x2a7a4c, 0x3e9150],
                [0xf4f7ee, 0xddebcb, 0xc4dea4, 0xa6cc7c],
                true,
            ),
            Self::LuloEmber => (
                "Ember",
                [0x120907, 0x2a0f07, 0xa8401a, 0xf08a24],
                [0xfff8f0, 0xfbead8, 0xf5c39c, 0xf7a260],
                true,
            ),
            Self::LuloDusk => (
                "Dusk",
                [0x061016, 0x0b1d24, 0x4a2c1e, 0xf08a24],
                [0xd8e8e4, 0xe6efea, 0xfae0c4, 0xf8c79a],
                true,
            ),
            Self::LuloMist => (
                "Mist",
                [0x232528, 0x1b1d20, 0x1f2a26, 0x131416],
                [0xf7f5f1, 0xefece6, 0xe5e7e0, 0xe6e1d9],
                true,
            ),
            Self::LuloNocturne => (
                "Nocturne",
                [0x07080a, 0x090b0c, 0x2e6b3a, 0x0b0d0d],
                [0xe9eaeb, 0xe2e3e3, 0xcde89a, 0xdcdddc],
                true,
            ),
            Self::Aurora => (
                "Aurora",
                [0x10162f, 0x3949ab, 0x22a6a1, 0xd96c9d],
                [0xdfe7ff, 0x8aa2e6, 0x8fdcd6, 0xf2c3d6],
                false,
            ),
            Self::Tide => (
                "Tide",
                [0x06283d, 0x1363df, 0x47b5ff, 0xe8f9fa],
                [0xdff1ff, 0x8fc6f0, 0x9fdcd6, 0xf5fafc],
                false,
            ),
            Self::Basalt => (
                "Basalt",
                [0x1b1b1f, 0x3a3a44, 0x6b4f3a, 0xc98a4b],
                [0xe8e6e2, 0xcfc9c0, 0xd8c3a6, 0xead6b8],
                false,
            ),
            Self::Monsoon => (
                "Monsoon",
                [0x0b1c1e, 0x1f4e4a, 0x4f7f6f, 0xb8c4b0],
                [0xe6efe9, 0xbfd4c9, 0x9fb8ac, 0xdce4da],
                false,
            ),
            Self::Paper => (
                "Paper",
                [0xf5efe6, 0xe6d9c7, 0xcbb99a, 0x8a7a5c],
                [0xfbf7f0, 0xf0e7d8, 0xdccbb0, 0xb3a184],
                false,
            ),
        };
        BuiltInMetadata {
            id: self,
            title,
            attribution: "Original procedural artwork by rmac",
            palette,
            light_palette,
            has_artwork,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BuiltInMetadata {
    pub id: BuiltInId,
    pub title: &'static str,
    pub attribution: &'static str,
    /// Original sRGB colors encoded as `0xRRGGBB` for a renderer-owned
    /// procedural gradient. No third-party bitmap is bundled.
    pub palette: [u32; 4],
    /// The light-appearance counterpart of `palette`; the wallpaper follows the
    /// system appearance (FEEL_SPEC.md §D.6).
    pub light_palette: [u32; 4],
    /// Whether the wallpaper is packaged artwork (a light and a dark image
    /// per [`ARTWORK_SIZES`] entry under [`PACKAGED_WALLPAPER_DIR`]) rather
    /// than a gradient drawn from `palette`.
    pub has_artwork: bool,
}

impl BuiltInMetadata {
    /// The gradient palette for the resolved appearance.
    pub fn palette_for(self, dark: bool) -> [u32; 4] {
        if dark {
            self.palette
        } else {
            self.light_palette
        }
    }

    /// The packaged image for this appearance that best covers a
    /// `width` x `height` physical output, if this wallpaper is artwork.
    pub fn artwork_path(self, dark: bool, width: u32, height: u32) -> Option<PathBuf> {
        let (width, height) = artwork_size(width, height);
        self.artwork_file(dark, &format!("{width}x{height}"))
    }

    /// The small System Settings gallery image for this appearance.
    pub fn thumbnail_path(self, dark: bool) -> Option<PathBuf> {
        self.artwork_file(dark, "thumbnail")
    }

    fn artwork_file(self, dark: bool, size: &str) -> Option<PathBuf> {
        if !self.has_artwork {
            return None;
        }
        let appearance = if dark { "dark" } else { "light" };
        Some(packaged_wallpaper_dir().join(format!("{}-{appearance}-{size}.jpg", self.id.id())))
    }
}

/// The installed wallpaper directory, or the development override when it
/// names a normalized absolute directory.
pub fn packaged_wallpaper_dir() -> PathBuf {
    std::env::var_os(PACKAGED_WALLPAPER_DIR_ENV)
        .map(PathBuf::from)
        .and_then(|path| validated_path(path).ok())
        .unwrap_or_else(|| PathBuf::from(PACKAGED_WALLPAPER_DIR))
}

/// The smallest packaged size that covers `width` x `height` on both axes,
/// or the largest when none does.
pub fn artwork_size(width: u32, height: u32) -> (u32, u32) {
    ARTWORK_SIZES
        .into_iter()
        .find(|&(w, h)| w >= width && h >= height)
        .unwrap_or(ARTWORK_SIZES[ARTWORK_SIZES.len() - 1])
}

#[derive(Clone, Eq, PartialEq)]
pub enum Source {
    BuiltIn(BuiltInId),
    File(PathBuf),
}

impl std::fmt::Debug for Source {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BuiltIn(id) => formatter.debug_tuple("BuiltIn").field(id).finish(),
            Self::File(_) => formatter.debug_tuple("File").field(&"<private>").finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceErrorKind {
    Empty,
    UnsupportedScheme,
    UnknownBuiltIn,
    RelativePath,
    UnnormalizedPath,
    InvalidFileUri,
}

pub fn parse_source(value: Option<&str>) -> Result<Source, SourceErrorKind> {
    let Some(value) = value else {
        return Ok(Source::BuiltIn(DEFAULT_BUILT_IN));
    };
    if value.trim().is_empty() {
        return Err(SourceErrorKind::Empty);
    }
    if let Some(id) = value.strip_prefix("builtin:") {
        return BuiltInId::parse(id)
            .map(Source::BuiltIn)
            .ok_or(SourceErrorKind::UnknownBuiltIn);
    }
    if value.starts_with("file:") {
        let url = url::Url::parse(value).map_err(|_| SourceErrorKind::InvalidFileUri)?;
        if url.scheme() != "file" || url.host_str().is_some() {
            return Err(SourceErrorKind::InvalidFileUri);
        }
        let path = url
            .to_file_path()
            .map_err(|_| SourceErrorKind::InvalidFileUri)?;
        return validated_path(path).map(Source::File);
    }
    if value.contains("://") {
        return Err(SourceErrorKind::UnsupportedScheme);
    }
    validated_path(PathBuf::from(value)).map(Source::File)
}

fn validated_path(path: PathBuf) -> Result<PathBuf, SourceErrorKind> {
    if !path.is_absolute() {
        return Err(SourceErrorKind::RelativePath);
    }
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(SourceErrorKind::UnnormalizedPath);
    }
    Ok(path)
}

pub fn file_path(source: &Source) -> Option<&Path> {
    match source {
        Source::File(path) => Some(path),
        Source::BuiltIn(_) => None,
    }
}
