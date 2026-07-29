//! Deterministic ownership and reconciliation for wallpaper background surfaces.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

pub const MAX_WALLPAPER_SURFACES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CommandId(u64);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SurfaceId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceOperation {
    Create,
    Present,
    Remove,
}

#[derive(Clone, Debug)]
pub enum CommandKind {
    Create {
        surface: SurfaceId,
        raster: rmac_wallpaper_image::RasterSurface,
    },
    Present {
        surface: SurfaceId,
        raster: rmac_wallpaper_image::RasterSurface,
    },
    Remove {
        surface: SurfaceId,
        output: rmac_compositor::OutputId,
    },
}

impl CommandKind {
    pub fn output(&self) -> &rmac_compositor::OutputId {
        match self {
            Self::Create { raster, .. } | Self::Present { raster, .. } => &raster.output,
            Self::Remove { output, .. } => output,
        }
    }

    pub fn surface(&self) -> SurfaceId {
        match self {
            Self::Create { surface, .. }
            | Self::Present { surface, .. }
            | Self::Remove { surface, .. } => *surface,
        }
    }

    pub fn operation(&self) -> SurfaceOperation {
        match self {
            Self::Create { .. } => SurfaceOperation::Create,
            Self::Present { .. } => SurfaceOperation::Present,
            Self::Remove { .. } => SurfaceOperation::Remove,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Command {
    id: CommandId,
    pub kind: CommandKind,
}

impl Command {
    pub fn id(&self) -> CommandId {
        self.id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandResult {
    Applied,
    Failed,
}

#[derive(Clone, Debug)]
pub struct AppliedSurface {
    pub output: rmac_compositor::OutputId,
    pub surface: SurfaceId,
    pub raster: rmac_wallpaper_image::RasterSurface,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceFailure {
    pub output: rmac_compositor::OutputId,
    pub operation: SurfaceOperation,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub requested_outputs: Vec<rmac_compositor::OutputId>,
    pub desired_outputs: Vec<rmac_compositor::OutputId>,
    /// Requested outputs with no valid raster and no prior accepted frame.
    pub unavailable_outputs: Vec<rmac_compositor::OutputId>,
    /// Requested outputs retaining their previous raster after a partial update.
    pub stale_outputs: Vec<rmac_compositor::OutputId>,
    pub applied: Vec<AppliedSurface>,
    pub pending: Option<Command>,
    pub failures: Vec<SurfaceFailure>,
}

#[derive(Clone, Debug, Default)]
pub struct Transition {
    pub snapshot: Snapshot,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleError {
    TooManySurfaces { count: usize },
    DuplicatePlanOutput,
    DuplicateRasterOutput,
    UnexpectedRasterOutput,
    InvalidPlan,
    InvalidRaster,
    CommandExhausted,
    SurfaceExhausted,
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManySurfaces { count } => write!(
                formatter,
                "the wallpaper requested {count} surfaces; the limit is {MAX_WALLPAPER_SURFACES}"
            ),
            Self::DuplicatePlanOutput => {
                formatter.write_str("the wallpaper plan repeats an output identity")
            }
            Self::DuplicateRasterOutput => {
                formatter.write_str("the wallpaper raster update repeats an output identity")
            }
            Self::UnexpectedRasterOutput => {
                formatter.write_str("the wallpaper raster update contains an unrequested output")
            }
            Self::InvalidPlan => formatter.write_str("the wallpaper surface plan is invalid"),
            Self::InvalidRaster => formatter.write_str("the wallpaper raster update is invalid"),
            Self::CommandExhausted => {
                formatter.write_str("the wallpaper command identity space is exhausted")
            }
            Self::SurfaceExhausted => {
                formatter.write_str("the wallpaper surface identity space is exhausted")
            }
        }
    }
}

impl std::error::Error for LifecycleError {}

#[derive(Clone)]
struct Applied {
    surface: SurfaceId,
    raster: rmac_wallpaper_image::RasterSurface,
}

#[derive(Default)]
pub struct Registry {
    requested: BTreeSet<rmac_compositor::OutputId>,
    desired: BTreeMap<rmac_compositor::OutputId, rmac_wallpaper_image::RasterSurface>,
    stale: BTreeSet<rmac_compositor::OutputId>,
    applied: BTreeMap<rmac_compositor::OutputId, Applied>,
    failures: BTreeMap<rmac_compositor::OutputId, SurfaceOperation>,
    pending: Option<Command>,
    next_command: u64,
    next_surface: u64,
}

impl Registry {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            requested_outputs: self.requested.iter().cloned().collect(),
            desired_outputs: self.desired.keys().cloned().collect(),
            unavailable_outputs: self
                .requested
                .iter()
                .filter(|output| !self.desired.contains_key(*output))
                .cloned()
                .collect(),
            stale_outputs: self.stale.iter().cloned().collect(),
            applied: self
                .applied
                .iter()
                .map(|(output, applied)| AppliedSurface {
                    output: output.clone(),
                    surface: applied.surface,
                    raster: applied.raster.clone(),
                })
                .collect(),
            pending: self.pending.clone(),
            failures: self
                .failures
                .iter()
                .map(|(output, operation)| SurfaceFailure {
                    output: output.clone(),
                    operation: *operation,
                })
                .collect(),
        }
    }

    /// Accept one complete plan plus its independently fallible rasterization.
    /// A missing raster retains the previous accepted desired frame for that
    /// still-requested output. A malformed whole update changes nothing.
    pub fn set_render_update(
        &mut self,
        plan: &rmac_wallpaper::Plan,
        rasterized: &rmac_wallpaper_image::Rasterized,
    ) -> Result<bool, LifecycleError> {
        if plan.surfaces.len() > MAX_WALLPAPER_SURFACES {
            return Err(LifecycleError::TooManySurfaces {
                count: plan.surfaces.len(),
            });
        }
        if rasterized.surfaces.len() > MAX_WALLPAPER_SURFACES {
            return Err(LifecycleError::TooManySurfaces {
                count: rasterized.surfaces.len(),
            });
        }
        let mut requested = BTreeSet::new();
        for surface in &plan.surfaces {
            if !valid_plan_surface(surface) {
                return Err(LifecycleError::InvalidPlan);
            }
            if !requested.insert(surface.output.clone()) {
                return Err(LifecycleError::DuplicatePlanOutput);
            }
        }

        let mut supplied = BTreeMap::new();
        for raster in &rasterized.surfaces {
            if !requested.contains(&raster.output) {
                return Err(LifecycleError::UnexpectedRasterOutput);
            }
            let planned = plan
                .surfaces
                .iter()
                .find(|surface| surface.output == raster.output)
                .expect("a requested raster has a plan surface");
            if !valid_raster(raster)
                || raster.logical_size != planned.logical_size
                || raster.scale != planned.scale
                || raster.fit != planned.fit
            {
                return Err(LifecycleError::InvalidRaster);
            }
            if supplied
                .insert(raster.output.clone(), raster.clone())
                .is_some()
            {
                return Err(LifecycleError::DuplicateRasterOutput);
            }
        }

        let mut desired = self
            .desired
            .iter()
            .filter(|(output, _)| requested.contains(*output))
            .map(|(output, raster)| (output.clone(), raster.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut stale = BTreeSet::new();
        for output in &requested {
            if let Some(raster) = supplied.remove(output) {
                desired.insert(output.clone(), raster);
            } else if desired.contains_key(output) {
                stale.insert(output.clone());
            }
        }
        let requested_changed = requested != self.requested;
        let desired_changed = !same_map(&desired, &self.desired);
        let stale_changed = stale != self.stale;
        if !requested_changed && !desired_changed && !stale_changed {
            return Ok(false);
        }
        let changed_outputs = self
            .desired
            .keys()
            .chain(desired.keys())
            .filter(
                |output| match (self.desired.get(*output), desired.get(*output)) {
                    (Some(left), Some(right)) => !same_raster(left, right),
                    (None, None) => false,
                    _ => true,
                },
            )
            .cloned()
            .collect::<BTreeSet<_>>();
        for output in changed_outputs {
            self.failures.remove(&output);
        }
        self.requested = requested;
        self.desired = desired;
        self.stale = stale;
        Ok(true)
    }

    pub fn set_update(&mut self, update: &crate::Update) -> Result<bool, LifecycleError> {
        match update {
            crate::Update::Render {
                plan, rasterized, ..
            } => self.set_render_update(plan, rasterized),
            crate::Update::Health(_) => Ok(false),
        }
    }

    /// Yield one command at a time. Applied state changes only after the real
    /// background surface adapter acknowledges the command.
    pub fn next_command(&mut self) -> Result<Option<Command>, LifecycleError> {
        if self.pending.is_some() {
            return Ok(None);
        }
        let Some(kind) = self.next_kind()? else {
            return Ok(None);
        };
        let next_command = self
            .next_command
            .checked_add(1)
            .ok_or(LifecycleError::CommandExhausted)?;
        self.next_command = next_command;
        let command = Command {
            id: CommandId(next_command),
            kind,
        };
        self.pending = Some(command.clone());
        Ok(Some(command))
    }

    pub fn finish(&mut self, id: CommandId, result: CommandResult) -> Transition {
        let Some(command) = self
            .pending
            .as_ref()
            .filter(|command| command.id == id)
            .cloned()
        else {
            return Transition {
                snapshot: self.snapshot(),
                visible: false,
            };
        };
        self.pending = None;
        let output = command.kind.output().clone();
        match result {
            CommandResult::Applied => {
                self.failures.remove(&output);
                match command.kind {
                    CommandKind::Create { surface, raster }
                    | CommandKind::Present { surface, raster } => {
                        self.applied.insert(output, Applied { surface, raster });
                    }
                    CommandKind::Remove { .. } => {
                        self.applied.remove(&output);
                    }
                }
            }
            CommandResult::Failed => {
                if self.failure_is_relevant(&command.kind) {
                    self.failures.insert(output, command.kind.operation());
                }
            }
        }
        Transition {
            snapshot: self.snapshot(),
            visible: true,
        }
    }

    /// Cancel only work known not to have touched the compositor. Uncertain
    /// partial work must be failed and reconciled explicitly instead.
    pub fn cancel(&mut self, id: CommandId) -> Transition {
        let visible = self
            .pending
            .as_ref()
            .is_some_and(|command| command.id == id);
        if visible {
            self.pending = None;
        }
        Transition {
            snapshot: self.snapshot(),
            visible,
        }
    }

    pub fn retry(&mut self, output: &rmac_compositor::OutputId) -> Transition {
        let visible = self.failures.remove(output).is_some();
        Transition {
            snapshot: self.snapshot(),
            visible,
        }
    }

    /// Reconcile an authoritative compositor close. Desired raster state is
    /// retained so the next command recreates the surface with a fresh
    /// physical identity.
    pub fn surface_closed(&mut self, surface: SurfaceId) -> Transition {
        let pending_output = self
            .pending
            .as_ref()
            .filter(|command| command.kind.surface() == surface)
            .map(|command| command.kind.output().clone());
        if pending_output.is_some() {
            self.pending = None;
        }
        let applied_output = self
            .applied
            .iter()
            .find_map(|(output, applied)| (applied.surface == surface).then(|| output.clone()));
        if let Some(output) = applied_output.as_ref() {
            self.applied.remove(output);
        }
        let mut visible = pending_output.is_some() || applied_output.is_some();
        for output in [pending_output.as_ref(), applied_output.as_ref()]
            .into_iter()
            .flatten()
        {
            visible |= self.failures.remove(output).is_some();
        }
        Transition {
            snapshot: self.snapshot(),
            visible,
        }
    }

    fn next_kind(&mut self) -> Result<Option<CommandKind>, LifecycleError> {
        if let Some((output, applied)) = self.applied.iter().find(|(output, _)| {
            !self.desired.contains_key(*output) && !self.failures.contains_key(*output)
        }) {
            return Ok(Some(CommandKind::Remove {
                surface: applied.surface,
                output: output.clone(),
            }));
        }
        if let Some((output, desired)) = self.desired.iter().find(|(output, desired)| {
            self.applied
                .get(*output)
                .is_some_and(|applied| !same_raster(&applied.raster, desired))
                && !self.failures.contains_key(*output)
        }) {
            return Ok(Some(CommandKind::Present {
                surface: self
                    .applied
                    .get(output)
                    .expect("a presentation update has an applied surface")
                    .surface,
                raster: desired.clone(),
            }));
        }
        if let Some((_, desired)) = self.desired.iter().find(|(output, _)| {
            !self.applied.contains_key(*output) && !self.failures.contains_key(*output)
        }) {
            let next_surface = self
                .next_surface
                .checked_add(1)
                .ok_or(LifecycleError::SurfaceExhausted)?;
            self.next_surface = next_surface;
            return Ok(Some(CommandKind::Create {
                surface: SurfaceId(next_surface),
                raster: desired.clone(),
            }));
        }
        Ok(None)
    }

    fn failure_is_relevant(&self, kind: &CommandKind) -> bool {
        match kind {
            CommandKind::Create { raster, .. } => {
                self.desired
                    .get(&raster.output)
                    .is_some_and(|desired| same_raster(desired, raster))
                    && !self.applied.contains_key(&raster.output)
            }
            CommandKind::Present { surface, raster } => {
                self.desired
                    .get(&raster.output)
                    .is_some_and(|desired| same_raster(desired, raster))
                    && self
                        .applied
                        .get(&raster.output)
                        .is_some_and(|applied| applied.surface == *surface)
            }
            CommandKind::Remove { surface, output } => {
                !self.desired.contains_key(output)
                    && self
                        .applied
                        .get(output)
                        .is_some_and(|applied| applied.surface == *surface)
            }
        }
    }
}

fn same_map(
    left: &BTreeMap<rmac_compositor::OutputId, rmac_wallpaper_image::RasterSurface>,
    right: &BTreeMap<rmac_compositor::OutputId, rmac_wallpaper_image::RasterSurface>,
) -> bool {
    left.len() == right.len()
        && left.iter().all(|(output, raster)| {
            right
                .get(output)
                .is_some_and(|candidate| same_raster(raster, candidate))
        })
}

fn same_raster(
    left: &rmac_wallpaper_image::RasterSurface,
    right: &rmac_wallpaper_image::RasterSurface,
) -> bool {
    left.output == right.output
        && left.logical_size == right.logical_size
        && left.scale == right.scale
        && left.fit == right.fit
        && left.layout == right.layout
        && Arc::ptr_eq(&left.image, &right.image)
}

fn valid_plan_surface(surface: &rmac_wallpaper::Surface) -> bool {
    !surface.output.0.is_empty()
        && surface.logical_size.is_valid()
        && surface.logical_size.width > 0.0
        && surface.logical_size.height > 0.0
        && surface.scale.is_finite()
        && surface.scale > 0.0
}

fn valid_raster(surface: &rmac_wallpaper_image::RasterSurface) -> bool {
    let pixels = u64::from(surface.image.width).checked_mul(u64::from(surface.image.height));
    let expected = pixels.and_then(|pixels| pixels.checked_mul(4));
    let destination = surface.layout.destination;
    !surface.output.0.is_empty()
        && surface.logical_size.is_valid()
        && surface.logical_size.width > 0.0
        && surface.logical_size.height > 0.0
        && surface.scale.is_finite()
        && surface.scale > 0.0
        && surface.image.width > 0
        && surface.image.height > 0
        && surface.image.width <= rmac_wallpaper_image::MAX_DIMENSION
        && surface.image.height <= rmac_wallpaper_image::MAX_DIMENSION
        && pixels.is_some_and(|pixels| pixels <= rmac_wallpaper_image::MAX_PIXELS)
        && expected == Some(surface.image.rgba.len() as u64)
        && destination.x.is_finite()
        && destination.y.is_finite()
        && destination.width.is_finite()
        && destination.height.is_finite()
        && destination.width > 0.0
        && destination.height > 0.0
        && rmac_wallpaper::layout(
            surface.fit,
            rmac_compositor::PhysicalSize {
                width: surface.image.width,
                height: surface.image.height,
            },
            surface.logical_size,
            surface.scale,
        ) == Ok(surface.layout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(outputs: &[&str]) -> rmac_wallpaper::Plan {
        rmac_wallpaper::Plan {
            surfaces: outputs
                .iter()
                .map(|output| rmac_wallpaper::Surface {
                    output: rmac_compositor::OutputId::from(*output),
                    logical_size: rmac_compositor::LogicalSize {
                        width: 1920.0,
                        height: 1080.0,
                    },
                    scale: 1.0,
                    fit: rmac_shell_settings::WallpaperFit::Fill,
                    source: rmac_wallpaper::Source::BuiltIn(rmac_wallpaper::DEFAULT_BUILT_IN),
                })
                .collect(),
            issues: Vec::new(),
        }
    }

    fn raster(output: &str, revision: u8) -> rmac_wallpaper_image::RasterSurface {
        let logical_size = rmac_compositor::LogicalSize {
            width: 1920.0,
            height: 1080.0,
        };
        let physical_size = rmac_compositor::PhysicalSize {
            width: 1,
            height: 1,
        };
        rmac_wallpaper_image::RasterSurface {
            output: rmac_compositor::OutputId::from(output),
            logical_size,
            scale: 1.0,
            fit: rmac_shell_settings::WallpaperFit::Fill,
            layout: rmac_wallpaper::layout(
                rmac_shell_settings::WallpaperFit::Fill,
                physical_size,
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

    fn rasterized(
        rasters: Vec<rmac_wallpaper_image::RasterSurface>,
    ) -> rmac_wallpaper_image::Rasterized {
        rmac_wallpaper_image::Rasterized {
            surfaces: rasters,
            issues: Vec::new(),
        }
    }

    fn apply_next(registry: &mut Registry) -> Command {
        let command = registry.next_command().unwrap().unwrap();
        registry.finish(command.id(), CommandResult::Applied);
        command
    }

    #[test]
    fn initial_surfaces_create_in_stable_output_order() {
        let mut registry = Registry::default();
        registry
            .set_render_update(
                &plan(&["B", "A"]),
                &rasterized(vec![raster("B", 1), raster("A", 1)]),
            )
            .unwrap();
        let first = apply_next(&mut registry);
        let second = apply_next(&mut registry);
        assert_eq!(first.kind.output(), &rmac_compositor::OutputId::from("A"));
        assert_eq!(second.kind.output(), &rmac_compositor::OutputId::from("B"));
        assert!(matches!(first.kind, CommandKind::Create { .. }));
        assert_eq!(registry.snapshot().applied.len(), 2);
    }

    #[test]
    fn hotplug_removes_before_creating_and_reuses_no_physical_identity() {
        let mut registry = Registry::default();
        registry
            .set_render_update(&plan(&["A"]), &rasterized(vec![raster("A", 1)]))
            .unwrap();
        let original = apply_next(&mut registry);
        registry
            .set_render_update(&plan(&["B"]), &rasterized(vec![raster("B", 1)]))
            .unwrap();
        let removed = apply_next(&mut registry);
        let created = apply_next(&mut registry);
        assert!(matches!(removed.kind, CommandKind::Remove { .. }));
        assert!(matches!(created.kind, CommandKind::Create { .. }));
        assert_ne!(original.kind.surface(), created.kind.surface());
    }

    #[test]
    fn new_raster_presents_on_the_same_surface_identity() {
        let mut registry = Registry::default();
        registry
            .set_render_update(&plan(&["A"]), &rasterized(vec![raster("A", 1)]))
            .unwrap();
        let created = apply_next(&mut registry);
        registry
            .set_render_update(&plan(&["A"]), &rasterized(vec![raster("A", 2)]))
            .unwrap();
        let presented = apply_next(&mut registry);
        assert!(matches!(presented.kind, CommandKind::Present { .. }));
        assert_eq!(created.kind.surface(), presented.kind.surface());
    }

    #[test]
    fn compositor_close_recreates_desired_surface_with_fresh_identity() {
        let mut registry = Registry::default();
        registry
            .set_render_update(&plan(&["A"]), &rasterized(vec![raster("A", 1)]))
            .unwrap();
        let created = apply_next(&mut registry);
        let old_surface = created.kind.surface();
        assert!(registry.surface_closed(old_surface).visible);
        assert!(registry.snapshot().applied.is_empty());
        assert!(!registry.surface_closed(old_surface).visible);

        let replacement = apply_next(&mut registry);
        assert!(matches!(replacement.kind, CommandKind::Create { .. }));
        assert_ne!(replacement.kind.surface(), old_surface);
    }

    #[test]
    fn partial_raster_update_retains_last_good_and_reports_stale_or_unavailable() {
        let mut registry = Registry::default();
        registry
            .set_render_update(&plan(&["A"]), &rasterized(vec![raster("A", 1)]))
            .unwrap();
        apply_next(&mut registry);
        registry
            .set_render_update(&plan(&["A", "B"]), &rasterized(Vec::new()))
            .unwrap();
        let snapshot = registry.snapshot();
        assert_eq!(
            snapshot.stale_outputs,
            [rmac_compositor::OutputId::from("A")]
        );
        assert_eq!(
            snapshot.unavailable_outputs,
            [rmac_compositor::OutputId::from("B")]
        );
        assert!(registry.next_command().unwrap().is_none());
        assert_eq!(snapshot.applied.len(), 1);
    }

    #[test]
    fn changed_desired_state_supersedes_pending_after_acknowledgement() {
        let mut registry = Registry::default();
        registry
            .set_render_update(&plan(&["A"]), &rasterized(vec![raster("A", 1)]))
            .unwrap();
        let create = registry.next_command().unwrap().unwrap();
        registry
            .set_render_update(&plan(&["A"]), &rasterized(vec![raster("A", 2)]))
            .unwrap();
        registry.finish(create.id(), CommandResult::Applied);
        let present = apply_next(&mut registry);
        assert!(matches!(present.kind, CommandKind::Present { .. }));
        assert_eq!(present.kind.surface(), create.kind.surface());
    }

    #[test]
    fn failure_blocks_only_one_output_until_retry_or_changed_raster() {
        let mut registry = Registry::default();
        registry
            .set_render_update(
                &plan(&["A", "B"]),
                &rasterized(vec![raster("A", 1), raster("B", 1)]),
            )
            .unwrap();
        let failed = registry.next_command().unwrap().unwrap();
        registry.finish(failed.id(), CommandResult::Failed);
        let other = apply_next(&mut registry);
        assert_eq!(other.kind.output(), &rmac_compositor::OutputId::from("B"));
        assert!(registry.next_command().unwrap().is_none());
        assert!(
            registry
                .retry(&rmac_compositor::OutputId::from("A"))
                .visible
        );
        assert_eq!(
            apply_next(&mut registry).kind.output(),
            failed.kind.output()
        );

        registry
            .set_render_update(
                &plan(&["A", "B"]),
                &rasterized(vec![raster("A", 2), raster("B", 1)]),
            )
            .unwrap();
        let changed = registry.next_command().unwrap().unwrap();
        registry.finish(changed.id(), CommandResult::Failed);
        assert_eq!(registry.snapshot().failures.len(), 1);
        registry
            .set_render_update(
                &plan(&["A", "B"]),
                &rasterized(vec![raster("A", 3), raster("B", 1)]),
            )
            .unwrap();
        assert!(registry.snapshot().failures.is_empty());
        assert_eq!(
            apply_next(&mut registry).kind.output(),
            changed.kind.output()
        );
    }

    #[test]
    fn failed_superseded_command_does_not_block_newer_work() {
        let mut registry = Registry::default();
        registry
            .set_render_update(&plan(&["A"]), &rasterized(vec![raster("A", 1)]))
            .unwrap();
        let stale = registry.next_command().unwrap().unwrap();
        registry
            .set_render_update(&plan(&["A"]), &rasterized(vec![raster("A", 2)]))
            .unwrap();

        registry.finish(stale.id(), CommandResult::Failed);

        assert!(registry.snapshot().failures.is_empty());
        let current = apply_next(&mut registry);
        assert!(matches!(current.kind, CommandKind::Create { .. }));
    }

    #[test]
    fn malformed_updates_preserve_last_accepted_desired_state() {
        let mut registry = Registry::default();
        let accepted_plan = plan(&["A"]);
        let accepted_raster = rasterized(vec![raster("A", 1)]);
        registry
            .set_render_update(&accepted_plan, &accepted_raster)
            .unwrap();

        let duplicate_plan = plan(&["A", "A"]);
        assert_eq!(
            registry.set_render_update(&duplicate_plan, &rasterized(Vec::new())),
            Err(LifecycleError::DuplicatePlanOutput)
        );
        assert_eq!(
            registry.set_render_update(
                &accepted_plan,
                &rasterized(vec![raster("A", 1), raster("A", 2)])
            ),
            Err(LifecycleError::DuplicateRasterOutput)
        );
        assert_eq!(
            registry.set_render_update(&accepted_plan, &rasterized(vec![raster("B", 1)])),
            Err(LifecycleError::UnexpectedRasterOutput)
        );
        let mut invalid = raster("A", 1);
        invalid.image = Arc::new(rmac_wallpaper_image::Decoded {
            width: 2,
            height: 2,
            rgba: Arc::from([0_u8; 4]),
        });
        assert_eq!(
            registry.set_render_update(&accepted_plan, &rasterized(vec![invalid])),
            Err(LifecycleError::InvalidRaster)
        );
        let mut invalid_plan = accepted_plan.clone();
        invalid_plan.surfaces[0].scale = f64::NAN;
        assert_eq!(
            registry.set_render_update(&invalid_plan, &rasterized(Vec::new())),
            Err(LifecycleError::InvalidPlan)
        );
        let mut mismatched = raster("A", 1);
        mismatched.logical_size.width = 1280.0;
        assert_eq!(
            registry.set_render_update(&accepted_plan, &rasterized(vec![mismatched])),
            Err(LifecycleError::InvalidRaster)
        );
        let too_many = (0..=MAX_WALLPAPER_SURFACES)
            .map(|index| format!("output-{index}"))
            .collect::<Vec<_>>();
        let too_many_refs = too_many.iter().map(String::as_str).collect::<Vec<_>>();
        assert_eq!(
            registry.set_render_update(&plan(&too_many_refs), &rasterized(Vec::new())),
            Err(LifecycleError::TooManySurfaces {
                count: MAX_WALLPAPER_SURFACES + 1
            })
        );
        assert_eq!(
            registry.snapshot().desired_outputs,
            [rmac_compositor::OutputId::from("A")]
        );
    }

    #[test]
    fn cancellation_and_stale_completion_cannot_mutate_newer_work() {
        let mut registry = Registry::default();
        registry
            .set_render_update(&plan(&["A"]), &rasterized(vec![raster("A", 1)]))
            .unwrap();
        let cancelled = registry.next_command().unwrap().unwrap();
        assert!(registry.cancel(cancelled.id()).visible);
        let replacement = registry.next_command().unwrap().unwrap();
        assert!(
            !registry
                .finish(cancelled.id(), CommandResult::Applied)
                .visible
        );
        assert!(registry.snapshot().applied.is_empty());
        registry.finish(replacement.id(), CommandResult::Applied);
        assert_eq!(registry.snapshot().applied.len(), 1);
    }

    #[test]
    fn health_only_update_never_changes_surface_lifecycle() {
        let mut registry = Registry::default();
        assert!(!registry
            .set_update(&crate::Update::Health(crate::HealthSnapshot::default()))
            .unwrap());
        assert!(registry.next_command().unwrap().is_none());
    }
}
