use std::sync::Arc;

use crate::{Cache, Decoded, ErrorKind};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RasterSource {
    BuiltIn(rmac_wallpaper::BuiltInId),
    UserFile,
}

#[derive(Clone, Debug)]
pub struct RasterSurface {
    pub output: rmac_compositor::OutputId,
    pub logical_size: rmac_compositor::LogicalSize,
    pub scale: f64,
    pub fit: rmac_shell_settings::WallpaperFit,
    /// Actual path-free source after any per-output fallback.
    pub source: RasterSource,
    pub fallback: bool,
    pub layout: rmac_wallpaper::Layout,
    pub image: Arc<Decoded>,
}

#[derive(Debug, Default)]
pub struct Rasterized {
    pub surfaces: Vec<RasterSurface>,
    pub issues: Vec<RasterIssue>,
}

/// [`rasterize_for`] in the dark appearance.
pub fn rasterize(plan: &rmac_wallpaper::Plan, cache: &Cache) -> Rasterized {
    rasterize_for(plan, cache, true)
}

/// Resolve, decode, and lay out every output independently, drawing built-ins
/// in their light or dark form. Any custom-file, packaged-artwork, or codec
/// failure substitutes the file-free fallback built-in only on that output.
pub fn rasterize_for(plan: &rmac_wallpaper::Plan, cache: &Cache, dark: bool) -> Rasterized {
    let mut rasterized = Rasterized::default();
    for surface in &plan.surfaces {
        let target = physical_target(surface.logical_size, surface.scale);
        let mut fallback = plan
            .issues
            .iter()
            .any(|issue| issue.output == surface.output);
        let (mut actual_source, resolved) = match rmac_wallpaper_system::resolve(&surface.source) {
            Ok(resolved) => {
                let source = match &resolved {
                    rmac_wallpaper_system::ResolvedSource::BuiltIn(metadata) => {
                        RasterSource::BuiltIn(metadata.id)
                    }
                    rmac_wallpaper_system::ResolvedSource::File(_) => RasterSource::UserFile,
                };
                (source, resolved)
            }
            Err(error) => {
                fallback = true;
                rasterized.issues.push(RasterIssue {
                    output: surface.output.clone(),
                    kind: RasterIssueKind::Resolve(error.kind),
                });
                (
                    RasterSource::BuiltIn(rmac_wallpaper::FALLBACK_BUILT_IN),
                    rmac_wallpaper_system::ResolvedSource::BuiltIn(
                        rmac_wallpaper::FALLBACK_BUILT_IN.metadata(),
                    ),
                )
            }
        };
        let image = match cache.get_or_decode_for(resolved, target, dark) {
            Ok(image) => image,
            Err(error) => {
                fallback = true;
                actual_source = RasterSource::BuiltIn(rmac_wallpaper::FALLBACK_BUILT_IN);
                rasterized.issues.push(RasterIssue {
                    output: surface.output.clone(),
                    kind: RasterIssueKind::Decode(error.kind),
                });
                match cache.get_or_decode_for(
                    rmac_wallpaper_system::ResolvedSource::BuiltIn(
                        rmac_wallpaper::FALLBACK_BUILT_IN.metadata(),
                    ),
                    target,
                    dark,
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
            source: actual_source,
            fallback,
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
