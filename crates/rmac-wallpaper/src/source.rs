use std::path::{Component, Path, PathBuf};

pub const DEFAULT_BUILT_IN: BuiltInId = BuiltInId::Aurora;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BuiltInId {
    Aurora,
    Tide,
    Basalt,
    Monsoon,
    Paper,
}

impl BuiltInId {
    /// Every original built-in, in picker order. `Aurora` is the default.
    pub const ALL: [BuiltInId; 5] = [
        BuiltInId::Aurora,
        BuiltInId::Tide,
        BuiltInId::Basalt,
        BuiltInId::Monsoon,
        BuiltInId::Paper,
    ];

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "rmac-aurora" => Some(Self::Aurora),
            "rmac-tide" => Some(Self::Tide),
            "rmac-basalt" => Some(Self::Basalt),
            "rmac-monsoon" => Some(Self::Monsoon),
            "rmac-paper" => Some(Self::Paper),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Aurora => "rmac-aurora",
            Self::Tide => "rmac-tide",
            Self::Basalt => "rmac-basalt",
            Self::Monsoon => "rmac-monsoon",
            Self::Paper => "rmac-paper",
        }
    }

    pub fn metadata(self) -> BuiltInMetadata {
        // Original sRGB four-stop palettes; the renderer draws a diagonal
        // gradient, so no third-party bitmap is ever bundled (FEEL_SPEC.md §D.6).
        let (title, palette, light_palette) = match self {
            Self::Aurora => (
                "Aurora",
                [0x10162f, 0x3949ab, 0x22a6a1, 0xd96c9d],
                [0xdfe7ff, 0x8aa2e6, 0x8fdcd6, 0xf2c3d6],
            ),
            Self::Tide => (
                "Tide",
                [0x06283d, 0x1363df, 0x47b5ff, 0xe8f9fa],
                [0xdff1ff, 0x8fc6f0, 0x9fdcd6, 0xf5fafc],
            ),
            Self::Basalt => (
                "Basalt",
                [0x1b1b1f, 0x3a3a44, 0x6b4f3a, 0xc98a4b],
                [0xe8e6e2, 0xcfc9c0, 0xd8c3a6, 0xead6b8],
            ),
            Self::Monsoon => (
                "Monsoon",
                [0x0b1c1e, 0x1f4e4a, 0x4f7f6f, 0xb8c4b0],
                [0xe6efe9, 0xbfd4c9, 0x9fb8ac, 0xdce4da],
            ),
            Self::Paper => (
                "Paper",
                [0xf5efe6, 0xe6d9c7, 0xcbb99a, 0x8a7a5c],
                [0xfbf7f0, 0xf0e7d8, 0xdccbb0, 0xb3a184],
            ),
        };
        BuiltInMetadata {
            id: self,
            title,
            attribution: "Original procedural artwork by rmac",
            palette,
            light_palette,
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
