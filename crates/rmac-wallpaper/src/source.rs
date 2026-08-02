use std::path::{Component, Path, PathBuf};

pub const DEFAULT_BUILT_IN: BuiltInId = BuiltInId::Aurora;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BuiltInId {
    Aurora,
}

impl BuiltInId {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "rmac-aurora" => Some(Self::Aurora),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Aurora => "rmac-aurora",
        }
    }

    pub fn metadata(self) -> BuiltInMetadata {
        match self {
            Self::Aurora => BuiltInMetadata {
                id: self,
                title: "Aurora",
                attribution: "Original procedural artwork by rmac",
                palette: [0x10162f, 0x3949ab, 0x22a6a1, 0xd96c9d],
            },
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
