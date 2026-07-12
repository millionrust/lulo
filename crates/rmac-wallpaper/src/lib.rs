//! Framework-neutral wallpaper source, output planning, and fit geometry.

pub mod portal;
pub mod transition;

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Issue {
    pub output: rmac_compositor::OutputId,
    pub kind: SourceErrorKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub output: rmac_compositor::OutputId,
    pub logical_size: rmac_compositor::LogicalSize,
    pub scale: f64,
    pub fit: rmac_shell_settings::WallpaperFit,
    pub source: Source,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub surfaces: Vec<Surface>,
    pub issues: Vec<Issue>,
}

/// Build one background surface per enabled, geometrically valid output.
/// Invalid per-output sources fall back locally and never blank other outputs.
pub fn plan(
    settings: &rmac_shell_settings::WallpaperSettings,
    outputs: &[rmac_compositor::Output],
) -> Plan {
    let mut enabled: Vec<_> = outputs
        .iter()
        .filter_map(|output| {
            let logical = output.logical.as_ref()?;
            (output.current_mode.is_some()
                && logical.size.is_valid()
                && logical.size.width > 0.0
                && logical.size.height > 0.0
                && logical.scale.is_finite()
                && logical.scale > 0.0)
                .then_some((output, logical))
        })
        .collect();
    enabled.sort_by(|left, right| left.0.id.cmp(&right.0.id));

    let mut plan = Plan::default();
    for (output, logical) in enabled {
        let selection = settings
            .per_output
            .get(&output.id.0)
            .unwrap_or(&settings.default);
        let source = match parse_source(selection.source.as_deref()) {
            Ok(source) => source,
            Err(kind) => {
                plan.issues.push(Issue {
                    output: output.id.clone(),
                    kind,
                });
                Source::BuiltIn(DEFAULT_BUILT_IN)
            }
        };
        plan.surfaces.push(Surface {
            output: output.id.clone(),
            logical_size: logical.size,
            scale: logical.scale,
            fit: selection.fit,
            source,
        });
    }
    plan
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layout {
    pub destination: Rect,
    pub tiled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutError {
    InvalidImage,
    InvalidViewport,
    InvalidScale,
}

pub fn layout(
    fit: rmac_shell_settings::WallpaperFit,
    image: rmac_compositor::PhysicalSize,
    viewport: rmac_compositor::LogicalSize,
    output_scale: f64,
) -> Result<Layout, LayoutError> {
    if image.width == 0 || image.height == 0 {
        return Err(LayoutError::InvalidImage);
    }
    if !viewport.is_valid() || viewport.width <= 0.0 || viewport.height <= 0.0 {
        return Err(LayoutError::InvalidViewport);
    }
    if !output_scale.is_finite() || output_scale <= 0.0 {
        return Err(LayoutError::InvalidScale);
    }
    let image_width = f64::from(image.width) / output_scale;
    let image_height = f64::from(image.height) / output_scale;
    let (width, height, tiled) = match fit {
        rmac_shell_settings::WallpaperFit::Fill => {
            let scale = (viewport.width / image_width).max(viewport.height / image_height);
            (image_width * scale, image_height * scale, false)
        }
        rmac_shell_settings::WallpaperFit::Fit => {
            let scale = (viewport.width / image_width).min(viewport.height / image_height);
            (image_width * scale, image_height * scale, false)
        }
        rmac_shell_settings::WallpaperFit::Stretch => (viewport.width, viewport.height, false),
        rmac_shell_settings::WallpaperFit::Center => (image_width, image_height, false),
        rmac_shell_settings::WallpaperFit::Tile => (image_width, image_height, true),
    };
    Ok(Layout {
        destination: Rect {
            x: (viewport.width - width) / 2.0,
            y: (viewport.height - height) / 2.0,
            width,
            height,
        },
        tiled,
    })
}

pub fn file_path(source: &Source) -> Option<&Path> {
    match source {
        Source::File(path) => Some(path),
        Source::BuiltIn(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(id: &str, enabled: bool, scale: f64) -> rmac_compositor::Output {
        rmac_compositor::Output {
            id: id.into(),
            make: "Test".into(),
            model: "Display".into(),
            serial: None,
            physical_size_mm: None,
            modes: vec![rmac_compositor::OutputMode {
                physical_size: rmac_compositor::PhysicalSize {
                    width: 2560,
                    height: 1440,
                },
                refresh_millihz: 60_000,
                preferred: true,
            }],
            current_mode: enabled.then_some(0),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: enabled.then_some(rmac_compositor::LogicalOutput {
                position: Default::default(),
                size: rmac_compositor::LogicalSize {
                    width: 1280.0,
                    height: 720.0,
                },
                scale,
                transform: "normal".into(),
            }),
        }
    }

    #[test]
    fn source_parser_accepts_original_builtin_and_safe_local_forms_only() {
        assert_eq!(parse_source(None), Ok(Source::BuiltIn(BuiltInId::Aurora)));
        assert_eq!(
            parse_source(Some("builtin:rmac-aurora")),
            Ok(Source::BuiltIn(BuiltInId::Aurora))
        );
        assert_eq!(
            parse_source(Some("file:///home/alex/Pictures/sky%20blue.png")),
            Ok(Source::File(PathBuf::from(
                "/home/alex/Pictures/sky blue.png"
            )))
        );
        assert!(matches!(
            parse_source(Some("https://example.com/wallpaper.jpg")),
            Err(SourceErrorKind::UnsupportedScheme)
        ));
        assert!(matches!(
            parse_source(Some("../secret.png")),
            Err(SourceErrorKind::RelativePath)
        ));
        let private = parse_source(Some("/home/alex/Private/wallpaper.png")).unwrap();
        assert!(!format!("{private:?}").contains("alex"));
        assert!(DEFAULT_BUILT_IN.metadata().attribution.contains("rmac"));
    }

    #[test]
    fn plan_is_per_output_sorted_hotplug_safe_and_locally_fallbacks() {
        let settings = rmac_shell_settings::WallpaperSettings {
            default: rmac_shell_settings::WallpaperSelection {
                source: Some("builtin:rmac-aurora".into()),
                fit: rmac_shell_settings::WallpaperFit::Fill,
            },
            per_output: [(
                "DP-2".into(),
                rmac_shell_settings::WallpaperSelection {
                    source: Some("https://invalid.example/image".into()),
                    fit: rmac_shell_settings::WallpaperFit::Fit,
                },
            )]
            .into_iter()
            .collect(),
        };
        let initial = plan(
            &settings,
            &[
                output("DP-2", true, 2.0),
                output("DP-1", true, 1.0),
                output("DP-3", false, 1.0),
            ],
        );
        assert_eq!(
            initial
                .surfaces
                .iter()
                .map(|surface| surface.output.0.as_str())
                .collect::<Vec<_>>(),
            ["DP-1", "DP-2"]
        );
        assert_eq!(initial.issues.len(), 1);
        assert_eq!(initial.issues[0].output.0, "DP-2");
        assert_eq!(
            initial.surfaces[1].source,
            Source::BuiltIn(BuiltInId::Aurora)
        );
        assert_eq!(
            initial.surfaces[1].fit,
            rmac_shell_settings::WallpaperFit::Fit
        );

        let unplugged = plan(&settings, &[output("DP-1", true, 1.0)]);
        assert_eq!(unplugged.surfaces.len(), 1);
        let replugged = plan(&settings, &[output("DP-2", true, 2.0)]);
        assert_eq!(
            replugged.surfaces[0].fit,
            rmac_shell_settings::WallpaperFit::Fit
        );
    }

    #[test]
    fn fit_geometry_covers_crop_letterbox_stretch_center_and_tile() {
        let image = rmac_compositor::PhysicalSize {
            width: 1000,
            height: 1000,
        };
        let viewport = rmac_compositor::LogicalSize {
            width: 1000.0,
            height: 500.0,
        };
        let fill = layout(
            rmac_shell_settings::WallpaperFit::Fill,
            image,
            viewport,
            1.0,
        )
        .unwrap();
        assert_eq!(
            fill.destination,
            Rect {
                x: 0.0,
                y: -250.0,
                width: 1000.0,
                height: 1000.0
            }
        );
        let fit = layout(rmac_shell_settings::WallpaperFit::Fit, image, viewport, 1.0).unwrap();
        assert_eq!(
            fit.destination,
            Rect {
                x: 250.0,
                y: 0.0,
                width: 500.0,
                height: 500.0
            }
        );
        let stretch = layout(
            rmac_shell_settings::WallpaperFit::Stretch,
            image,
            viewport,
            1.0,
        )
        .unwrap();
        assert_eq!(stretch.destination.width, viewport.width);
        assert_eq!(stretch.destination.height, viewport.height);
        assert_eq!(
            layout(
                rmac_shell_settings::WallpaperFit::Center,
                image,
                viewport,
                2.0
            )
            .unwrap()
            .destination,
            Rect {
                x: 250.0,
                y: 0.0,
                width: 500.0,
                height: 500.0
            }
        );
        assert!(
            layout(
                rmac_shell_settings::WallpaperFit::Tile,
                image,
                viewport,
                2.0
            )
            .unwrap()
            .tiled
        );
    }
}
