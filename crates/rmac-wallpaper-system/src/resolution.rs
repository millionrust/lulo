use crate::{open_file, Error, Resolution, ResolutionIssue, ResolvedSource, ResolvedSurface};

/// Resolve every planned output independently. One invalid custom file falls
/// back to the original built-in on that output and never removes peers.
pub fn resolve_plan(plan: &rmac_wallpaper::Plan) -> Resolution {
    let mut resolution = Resolution::default();
    for surface in &plan.surfaces {
        let source = match resolve(&surface.source) {
            Ok(source) => source,
            Err(error) => {
                resolution.issues.push(ResolutionIssue {
                    output: surface.output.clone(),
                    kind: error.kind,
                });
                ResolvedSource::BuiltIn(rmac_wallpaper::DEFAULT_BUILT_IN.metadata())
            }
        };
        resolution.surfaces.push(ResolvedSurface {
            output: surface.output.clone(),
            logical_size: surface.logical_size,
            scale: surface.scale,
            fit: surface.fit,
            source,
        });
    }
    resolution
}

pub fn resolve(source: &rmac_wallpaper::Source) -> Result<ResolvedSource, Error> {
    match source {
        rmac_wallpaper::Source::BuiltIn(id) => Ok(ResolvedSource::BuiltIn(id.metadata())),
        rmac_wallpaper::Source::File(path) => open_file(path).map(ResolvedSource::File),
    }
}
