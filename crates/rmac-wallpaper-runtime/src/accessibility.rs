//! Passive, path-free desktop semantics derived from applied wallpaper surfaces.

use std::collections::HashSet;
use std::fmt;

use crate::{session, surfaces, SourceHealth};

pub const DESKTOP_NAME: &str = "Desktop";
pub const ROOT_ID_PREFIX: &str = "desktop";
pub const IMAGE_ID_SUFFIX: &str = "wallpaper";
pub const USER_WALLPAPER_NAME: &str = "User wallpaper";
pub const MAX_OUTPUT_ID_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibleRole {
    Desktop,
    Image,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibleWallpaperSource {
    BuiltIn(rmac_wallpaper::BuiltInId),
    UserFile,
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleWallpaperImage {
    pub id: String,
    pub role: AccessibleRole,
    pub name: String,
    pub description: String,
    pub source: AccessibleWallpaperSource,
    pub fit: rmac_shell_settings::WallpaperFit,
    pub fallback: bool,
    pub stale: bool,
    pub update_failed: bool,
}

impl fmt::Debug for AccessibleWallpaperImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleWallpaperImage")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("name", &self.name)
            .field("description", &self.description)
            .field("source", &self.source)
            .field("fit", &self.fit)
            .field("fallback", &self.fallback)
            .field("stale", &self.stale)
            .field("update_failed", &self.update_failed)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleDesktopSurface {
    /// Stable only within this sorted complete snapshot; output identity stays private.
    pub ordinal: usize,
    pub root_id: String,
    pub role: AccessibleRole,
    pub name: &'static str,
    pub keyboard_interactive: bool,
    pub wallpaper: AccessibleWallpaperImage,
    pub reading_order: Vec<String>,
    pub focus_order: Vec<String>,
}

impl fmt::Debug for AccessibleDesktopSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleDesktopSurface")
            .field("ordinal", &self.ordinal)
            .field("root_id", &self.root_id)
            .field("role", &self.role)
            .field("name", &self.name)
            .field("keyboard_interactive", &self.keyboard_interactive)
            .field("wallpaper", &self.wallpaper)
            .field("reading_order", &self.reading_order)
            .field("focus_order", &self.focus_order)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct DesktopAccessibilitySnapshot {
    pub requested_surface_count: usize,
    pub unavailable_surface_count: usize,
    pub applied_surface_count: usize,
    pub host_ready: bool,
    pub loading: bool,
    pub degraded: bool,
    pub surfaces: Vec<AccessibleDesktopSurface>,
}

impl fmt::Debug for DesktopAccessibilitySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DesktopAccessibilitySnapshot")
            .field("requested_surface_count", &self.requested_surface_count)
            .field("unavailable_surface_count", &self.unavailable_surface_count)
            .field("applied_surface_count", &self.applied_surface_count)
            .field("host_ready", &self.host_ready)
            .field("loading", &self.loading)
            .field("degraded", &self.degraded)
            .field("surfaces", &self.surfaces)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibilityProjectionError {
    SurfaceLimit,
    InvalidOutput,
    DuplicateOutput,
    InvalidSnapshot,
    InvalidRaster,
}

pub fn project_accessibility(
    snapshot: &session::Snapshot,
) -> Result<DesktopAccessibilitySnapshot, AccessibilityProjectionError> {
    validate_snapshot(snapshot)?;
    let loading = !snapshot.host_ready
        || health_values(&snapshot.health).any(|health| matches!(health, SourceHealth::Starting));
    let health_degraded = health_values(&snapshot.health)
        .any(|health| matches!(health, SourceHealth::Unavailable { .. }));
    let degraded = health_degraded
        || snapshot.lifecycle_error.is_some()
        || !snapshot.surfaces.unavailable_outputs.is_empty()
        || !snapshot.surfaces.stale_outputs.is_empty()
        || !snapshot.surfaces.failures.is_empty();

    let stale = snapshot
        .surfaces
        .stale_outputs
        .iter()
        .collect::<HashSet<_>>();
    let failures = snapshot
        .surfaces
        .failures
        .iter()
        .map(|failure| &failure.output)
        .collect::<HashSet<_>>();
    let mut surfaces = Vec::with_capacity(snapshot.surfaces.applied.len());
    for (index, applied) in snapshot.surfaces.applied.iter().enumerate() {
        let ordinal = index + 1;
        let root_id = format!("{ROOT_ID_PREFIX}-{ordinal}");
        let image_id = format!("{root_id}-{IMAGE_ID_SUFFIX}");
        let source = match applied.raster.source {
            rmac_wallpaper_image::RasterSource::BuiltIn(id) => {
                AccessibleWallpaperSource::BuiltIn(id)
            }
            rmac_wallpaper_image::RasterSource::UserFile => AccessibleWallpaperSource::UserFile,
        };
        let name = match source {
            AccessibleWallpaperSource::BuiltIn(id) => {
                format!("{} wallpaper", id.metadata().title)
            }
            AccessibleWallpaperSource::UserFile => USER_WALLPAPER_NAME.into(),
        };
        let is_stale = stale.contains(&applied.output);
        let update_failed = failures.contains(&applied.output);
        let mut states = vec![fit_name(applied.raster.fit)];
        if applied.raster.fallback {
            states.push("using the Aurora fallback");
        }
        if is_stale {
            states.push("previous wallpaper retained while replacement is unavailable");
        }
        if update_failed {
            states.push("wallpaper update failed");
        }
        let description = format!("{name}, {}", states.join(", "));
        surfaces.push(AccessibleDesktopSurface {
            ordinal,
            root_id,
            role: AccessibleRole::Desktop,
            name: DESKTOP_NAME,
            keyboard_interactive: false,
            wallpaper: AccessibleWallpaperImage {
                id: image_id.clone(),
                role: AccessibleRole::Image,
                name,
                description,
                source,
                fit: applied.raster.fit,
                fallback: applied.raster.fallback,
                stale: is_stale,
                update_failed,
            },
            reading_order: vec![image_id],
            focus_order: Vec::new(),
        });
    }

    Ok(DesktopAccessibilitySnapshot {
        requested_surface_count: snapshot.surfaces.requested_outputs.len(),
        unavailable_surface_count: snapshot.surfaces.unavailable_outputs.len(),
        applied_surface_count: surfaces.len(),
        host_ready: snapshot.host_ready,
        loading,
        degraded,
        surfaces,
    })
}

fn validate_snapshot(snapshot: &session::Snapshot) -> Result<(), AccessibilityProjectionError> {
    let state = &snapshot.surfaces;
    for count in [
        state.requested_outputs.len(),
        state.desired_outputs.len(),
        state.unavailable_outputs.len(),
        state.stale_outputs.len(),
        state.applied.len(),
        state.failures.len(),
    ] {
        if count > surfaces::MAX_WALLPAPER_SURFACES {
            return Err(AccessibilityProjectionError::SurfaceLimit);
        }
    }
    validate_outputs(&state.requested_outputs)?;
    validate_outputs(&state.desired_outputs)?;
    validate_outputs(&state.unavailable_outputs)?;
    validate_outputs(&state.stale_outputs)?;
    let applied_outputs = state
        .applied
        .iter()
        .map(|applied| applied.output.clone())
        .collect::<Vec<_>>();
    validate_outputs(&applied_outputs)?;
    let failure_outputs = state
        .failures
        .iter()
        .map(|failure| failure.output.clone())
        .collect::<Vec<_>>();
    validate_outputs(&failure_outputs)?;

    if !state
        .desired_outputs
        .iter()
        .all(|output| state.requested_outputs.contains(output))
        || !state
            .stale_outputs
            .iter()
            .all(|output| state.desired_outputs.contains(output))
    {
        return Err(AccessibilityProjectionError::InvalidSnapshot);
    }
    let expected_unavailable = state
        .requested_outputs
        .iter()
        .filter(|output| !state.desired_outputs.contains(*output))
        .cloned()
        .collect::<Vec<_>>();
    if state.unavailable_outputs != expected_unavailable {
        return Err(AccessibilityProjectionError::InvalidSnapshot);
    }
    if !snapshot.host_ready && (!state.applied.is_empty() || state.pending.is_some()) {
        return Err(AccessibilityProjectionError::InvalidSnapshot);
    }
    for applied in &state.applied {
        if applied.output != applied.raster.output || !surfaces::valid_raster(&applied.raster) {
            return Err(AccessibilityProjectionError::InvalidRaster);
        }
    }
    if let Some(command) = &state.pending {
        validate_output(command.kind.output())?;
        match &command.kind {
            surfaces::CommandKind::Create { raster, .. }
            | surfaces::CommandKind::Present { raster, .. } => {
                if !surfaces::valid_raster(raster) {
                    return Err(AccessibilityProjectionError::InvalidRaster);
                }
            }
            surfaces::CommandKind::Remove { .. } => {}
        }
    }
    Ok(())
}

fn validate_outputs(
    outputs: &[rmac_compositor::OutputId],
) -> Result<(), AccessibilityProjectionError> {
    let mut previous: Option<&str> = None;
    for output in outputs {
        validate_output(output)?;
        if previous.is_some_and(|previous| previous >= output.0.as_str()) {
            return Err(AccessibilityProjectionError::DuplicateOutput);
        }
        previous = Some(output.0.as_str());
    }
    Ok(())
}

fn validate_output(output: &rmac_compositor::OutputId) -> Result<(), AccessibilityProjectionError> {
    if output.0.trim().is_empty()
        || output.0.len() > MAX_OUTPUT_ID_BYTES
        || output.0.chars().any(char::is_control)
    {
        return Err(AccessibilityProjectionError::InvalidOutput);
    }
    Ok(())
}

fn health_values(health: &crate::HealthSnapshot) -> impl Iterator<Item = &SourceHealth> {
    [&health.compositor, &health.settings, &health.files].into_iter()
}

const fn fit_name(fit: rmac_shell_settings::WallpaperFit) -> &'static str {
    match fit {
        rmac_shell_settings::WallpaperFit::Fill => "Fill",
        rmac_shell_settings::WallpaperFit::Fit => "Fit",
        rmac_shell_settings::WallpaperFit::Stretch => "Stretch",
        rmac_shell_settings::WallpaperFit::Center => "Center",
        rmac_shell_settings::WallpaperFit::Tile => "Tile",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::session::Session;

    fn plan_surface(
        output: &str,
        source: rmac_wallpaper::Source,
        fit: rmac_shell_settings::WallpaperFit,
    ) -> rmac_wallpaper::Surface {
        rmac_wallpaper::Surface {
            output: output.into(),
            logical_size: rmac_compositor::LogicalSize {
                width: 1920.0,
                height: 1080.0,
            },
            scale: 1.0,
            fit,
            source,
        }
    }

    fn raster(
        output: &str,
        source: rmac_wallpaper_image::RasterSource,
        fallback: bool,
        fit: rmac_shell_settings::WallpaperFit,
        revision: u8,
    ) -> rmac_wallpaper_image::RasterSurface {
        let logical_size = rmac_compositor::LogicalSize {
            width: 1920.0,
            height: 1080.0,
        };
        rmac_wallpaper_image::RasterSurface {
            output: output.into(),
            logical_size,
            scale: 1.0,
            fit,
            source,
            fallback,
            layout: rmac_wallpaper::layout(
                fit,
                rmac_compositor::PhysicalSize {
                    width: 1,
                    height: 1,
                },
                logical_size,
                1.0,
            )
            .unwrap(),
            image: Arc::new(rmac_wallpaper_image::Decoded {
                width: 1,
                height: 1,
                rgba: Arc::from([revision, revision, revision, 255]),
            }),
        }
    }

    fn update(
        surfaces: Vec<rmac_wallpaper::Surface>,
        rasters: Vec<rmac_wallpaper_image::RasterSurface>,
        issues: Vec<rmac_wallpaper_image::RasterIssue>,
    ) -> crate::Update {
        crate::Update::Render {
            plan: rmac_wallpaper::Plan {
                surfaces,
                issues: Vec::new(),
            },
            rasterized: rmac_wallpaper_image::Rasterized {
                surfaces: rasters,
                issues,
            },
            health: crate::HealthSnapshot {
                compositor: SourceHealth::Healthy,
                settings: SourceHealth::Healthy,
                files: SourceHealth::Healthy,
            },
        }
    }

    fn apply_all(session: &mut Session, update: &crate::Update) {
        let mut transition = session.apply(update);
        while let Some(command) = transition.command {
            transition = session.finish(command.id(), surfaces::CommandResult::Applied);
        }
    }

    #[test]
    fn applied_builtin_and_user_surfaces_are_passive_named_desktops() {
        let mut session = Session::default();
        session.host_ready();
        apply_all(
            &mut session,
            &update(
                vec![
                    plan_surface(
                        "Private-A",
                        rmac_wallpaper::Source::BuiltIn(rmac_wallpaper::DEFAULT_BUILT_IN),
                        rmac_shell_settings::WallpaperFit::Fill,
                    ),
                    plan_surface(
                        "Private-B",
                        rmac_wallpaper::Source::File("/private/wallpaper.png".into()),
                        rmac_shell_settings::WallpaperFit::Fit,
                    ),
                ],
                vec![
                    raster(
                        "Private-A",
                        rmac_wallpaper_image::RasterSource::BuiltIn(
                            rmac_wallpaper::DEFAULT_BUILT_IN,
                        ),
                        false,
                        rmac_shell_settings::WallpaperFit::Fill,
                        1,
                    ),
                    raster(
                        "Private-B",
                        rmac_wallpaper_image::RasterSource::UserFile,
                        false,
                        rmac_shell_settings::WallpaperFit::Fit,
                        2,
                    ),
                ],
                Vec::new(),
            ),
        );

        let projected = project_accessibility(&session.snapshot()).unwrap();
        assert_eq!(projected.applied_surface_count, 2);
        assert!(!projected.degraded);
        assert_eq!(projected.surfaces[0].name, DESKTOP_NAME);
        assert_eq!(projected.surfaces[0].wallpaper.name, "Lulo wallpaper");
        assert_eq!(projected.surfaces[1].wallpaper.name, USER_WALLPAPER_NAME);
        assert_eq!(
            projected.surfaces[1].wallpaper.fit,
            rmac_shell_settings::WallpaperFit::Fit
        );
        assert!(projected
            .surfaces
            .iter()
            .all(|surface| !surface.keyboard_interactive && surface.focus_order.is_empty()));
        let diagnostics = format!("{projected:?}");
        assert!(!diagnostics.contains("Private-A"));
        assert!(!diagnostics.contains("Private-B"));
        assert!(!diagnostics.contains("/private/wallpaper"));
    }

    #[test]
    fn retained_and_fallback_frames_expose_truthful_nonprivate_state() {
        let mut session = Session::default();
        session.host_ready();
        let planned = plan_surface(
            "Private-A",
            rmac_wallpaper::Source::File("/private/wallpaper.png".into()),
            rmac_shell_settings::WallpaperFit::Fill,
        );
        apply_all(
            &mut session,
            &update(
                vec![planned.clone()],
                vec![raster(
                    "Private-A",
                    rmac_wallpaper_image::RasterSource::UserFile,
                    false,
                    rmac_shell_settings::WallpaperFit::Fill,
                    1,
                )],
                Vec::new(),
            ),
        );
        apply_all(
            &mut session,
            &update(vec![planned.clone()], Vec::new(), Vec::new()),
        );
        let retained = project_accessibility(&session.snapshot()).unwrap();
        assert!(retained.degraded);
        assert!(retained.surfaces[0].wallpaper.stale);
        assert!(retained.surfaces[0]
            .wallpaper
            .description
            .contains("previous wallpaper retained"));

        apply_all(
            &mut session,
            &update(
                vec![planned],
                vec![raster(
                    "Private-A",
                    rmac_wallpaper_image::RasterSource::BuiltIn(rmac_wallpaper::FALLBACK_BUILT_IN),
                    true,
                    rmac_shell_settings::WallpaperFit::Fill,
                    2,
                )],
                vec![rmac_wallpaper_image::RasterIssue {
                    output: "Private-A".into(),
                    kind: rmac_wallpaper_image::RasterIssueKind::Decode(
                        rmac_wallpaper_image::ErrorKind::Decode,
                    ),
                }],
            ),
        );
        let fallback = project_accessibility(&session.snapshot()).unwrap();
        assert!(fallback.surfaces[0].wallpaper.fallback);
        assert!(!fallback.surfaces[0].wallpaper.stale);
        assert_eq!(fallback.surfaces[0].wallpaper.name, "Aurora wallpaper");
    }

    #[test]
    fn malformed_snapshot_and_mismatched_source_fail_closed() {
        let duplicate = session::Snapshot {
            health: Default::default(),
            surfaces: surfaces::Snapshot {
                requested_outputs: vec!["same".into(), "same".into()],
                ..Default::default()
            },
            lifecycle_error: None,
            host_ready: false,
        };
        assert_eq!(
            project_accessibility(&duplicate),
            Err(AccessibilityProjectionError::DuplicateOutput)
        );

        let oversized_id = "x".repeat(MAX_OUTPUT_ID_BYTES + 1);
        let oversized = session::Snapshot {
            health: Default::default(),
            surfaces: surfaces::Snapshot {
                requested_outputs: vec![oversized_id.as_str().into()],
                ..Default::default()
            },
            lifecycle_error: None,
            host_ready: false,
        };
        assert_eq!(
            project_accessibility(&oversized),
            Err(AccessibilityProjectionError::InvalidOutput)
        );

        let mut session = Session::default();
        session.host_ready();
        let rejected = session.apply(&update(
            vec![plan_surface(
                "A",
                rmac_wallpaper::Source::BuiltIn(rmac_wallpaper::DEFAULT_BUILT_IN),
                rmac_shell_settings::WallpaperFit::Fill,
            )],
            vec![raster(
                "A",
                rmac_wallpaper_image::RasterSource::UserFile,
                false,
                rmac_shell_settings::WallpaperFit::Fill,
                1,
            )],
            Vec::new(),
        ));
        assert!(rejected.command.is_none());
        assert_eq!(
            rejected.snapshot.lifecycle_error,
            Some(surfaces::LifecycleError::InvalidRaster)
        );
        let projected = project_accessibility(&rejected.snapshot).unwrap();
        assert!(projected.degraded);
        assert!(projected.surfaces.is_empty());
    }
}
