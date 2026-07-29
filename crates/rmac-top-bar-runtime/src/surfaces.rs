//! Deterministic ownership and reconciliation for per-output top-bar surfaces.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const MAX_TOP_BAR_SURFACES: usize = 32;
const MAX_TEXT_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CommandId(u64);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SurfaceId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceOperation {
    Create,
    Reconfigure,
    Remove,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Description {
    pub surface: rmac_top_bar::Surface,
    pub content: rmac_top_bar::Content,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CommandKind {
    Create {
        surface: SurfaceId,
        description: Description,
    },
    Reconfigure {
        surface: SurfaceId,
        description: Description,
    },
    Remove {
        surface: SurfaceId,
        output: rmac_compositor::OutputId,
    },
}

impl CommandKind {
    pub fn output(&self) -> &rmac_compositor::OutputId {
        match self {
            Self::Create { description, .. } | Self::Reconfigure { description, .. } => {
                &description.surface.output
            }
            Self::Remove { output, .. } => output,
        }
    }

    pub fn surface(&self) -> SurfaceId {
        match self {
            Self::Create { surface, .. }
            | Self::Reconfigure { surface, .. }
            | Self::Remove { surface, .. } => *surface,
        }
    }

    pub fn operation(&self) -> SurfaceOperation {
        match self {
            Self::Create { .. } => SurfaceOperation::Create,
            Self::Reconfigure { .. } => SurfaceOperation::Reconfigure,
            Self::Remove { .. } => SurfaceOperation::Remove,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct AppliedSurface {
    pub output: rmac_compositor::OutputId,
    pub surface: SurfaceId,
    pub description: Description,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceFailure {
    pub output: rmac_compositor::OutputId,
    pub operation: SurfaceOperation,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub desired_outputs: Vec<rmac_compositor::OutputId>,
    pub applied: Vec<AppliedSurface>,
    pub pending: Option<Command>,
    pub failures: Vec<SurfaceFailure>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transition {
    pub snapshot: Snapshot,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleError {
    TooManySurfaces { count: usize },
    DuplicateOutput,
    InvalidSurface,
    InvalidContent,
    CommandExhausted,
    SurfaceExhausted,
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManySurfaces { count } => write!(
                formatter,
                "the top bar requested {count} surfaces; the limit is {MAX_TOP_BAR_SURFACES}"
            ),
            Self::DuplicateOutput => {
                formatter.write_str("the top-bar projection repeats an output identity")
            }
            Self::InvalidSurface => {
                formatter.write_str("the top-bar projection contains invalid surface geometry")
            }
            Self::InvalidContent => {
                formatter.write_str("the top-bar projection contains invalid content")
            }
            Self::CommandExhausted => {
                formatter.write_str("the top-bar command identity space is exhausted")
            }
            Self::SurfaceExhausted => {
                formatter.write_str("the top-bar surface identity space is exhausted")
            }
        }
    }
}

impl std::error::Error for LifecycleError {}

#[derive(Clone, Debug)]
struct Applied {
    surface: SurfaceId,
    description: Description,
}

#[derive(Default)]
pub struct Registry {
    desired: BTreeMap<rmac_compositor::OutputId, Description>,
    applied: BTreeMap<rmac_compositor::OutputId, Applied>,
    failures: BTreeMap<rmac_compositor::OutputId, SurfaceOperation>,
    pending: Option<Command>,
    next_command: u64,
    next_surface: u64,
}

impl Registry {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            desired_outputs: self.desired.keys().cloned().collect(),
            applied: self
                .applied
                .iter()
                .map(|(output, applied)| AppliedSurface {
                    output: output.clone(),
                    surface: applied.surface,
                    description: applied.description.clone(),
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

    /// Accept one complete renderer projection. Malformed projections preserve
    /// the last accepted desired and applied state.
    pub fn set_projection(
        &mut self,
        projection: &rmac_top_bar::Projection,
    ) -> Result<bool, LifecycleError> {
        if projection.surfaces.len() > MAX_TOP_BAR_SURFACES {
            return Err(LifecycleError::TooManySurfaces {
                count: projection.surfaces.len(),
            });
        }
        if !valid_content(&projection.content) {
            return Err(LifecycleError::InvalidContent);
        }
        let mut desired = BTreeMap::new();
        for surface in &projection.surfaces {
            if !valid_surface(surface) {
                return Err(LifecycleError::InvalidSurface);
            }
            let description = Description {
                surface: surface.clone(),
                content: projection.content.clone(),
            };
            if desired
                .insert(surface.output.clone(), description)
                .is_some()
            {
                return Err(LifecycleError::DuplicateOutput);
            }
        }
        if desired == self.desired {
            return Ok(false);
        }
        let changed_outputs = self
            .desired
            .keys()
            .chain(desired.keys())
            .filter(|output| self.desired.get(*output) != desired.get(*output))
            .cloned()
            .collect::<BTreeSet<_>>();
        for output in changed_outputs {
            self.failures.remove(&output);
        }
        self.desired = desired;
        Ok(true)
    }

    pub fn set_update(&mut self, update: &rmac_top_bar::Update) -> Result<bool, LifecycleError> {
        self.set_projection(&update.projection)
    }

    /// Yield at most one command. Applied state changes only after the real
    /// layer host acknowledges that exact command.
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
                    CommandKind::Create {
                        surface,
                        description,
                    }
                    | CommandKind::Reconfigure {
                        surface,
                        description,
                    } => {
                        self.applied.insert(
                            output,
                            Applied {
                                surface,
                                description,
                            },
                        );
                    }
                    CommandKind::Remove { .. } => {
                        self.applied.remove(&output);
                    }
                }
            }
            CommandResult::Failed if self.failure_is_relevant(&command.kind) => {
                self.failures.insert(output, command.kind.operation());
            }
            CommandResult::Failed => {}
        }
        Transition {
            snapshot: self.snapshot(),
            visible: true,
        }
    }

    /// Reconcile an authoritative compositor close. Any command naming that
    /// exact physical surface becomes stale, and a still-desired output will
    /// receive a new identity on the next command.
    pub fn surface_closed(&mut self, surface: SurfaceId) -> Transition {
        let output = self
            .applied
            .iter()
            .find_map(|(output, applied)| (applied.surface == surface).then(|| output.clone()));
        let pending_matches = self
            .pending
            .as_ref()
            .is_some_and(|command| command.kind.surface() == surface);
        if pending_matches {
            self.pending = None;
        }
        if let Some(output) = &output {
            self.applied.remove(output);
            self.failures.remove(output);
        }
        Transition {
            snapshot: self.snapshot(),
            visible: pending_matches || output.is_some(),
        }
    }

    /// Cancel only work proven not to have reached the platform.
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
                .is_some_and(|applied| applied.description != **desired)
                && !self.failures.contains_key(*output)
        }) {
            return Ok(Some(CommandKind::Reconfigure {
                surface: self
                    .applied
                    .get(output)
                    .expect("a reconfiguration has an applied surface")
                    .surface,
                description: desired.clone(),
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
                description: desired.clone(),
            }));
        }
        Ok(None)
    }

    fn failure_is_relevant(&self, kind: &CommandKind) -> bool {
        match kind {
            CommandKind::Create { description, .. } => {
                self.desired
                    .get(&description.surface.output)
                    .is_some_and(|desired| desired == description)
                    && !self.applied.contains_key(&description.surface.output)
            }
            CommandKind::Reconfigure {
                surface,
                description,
            } => {
                self.desired
                    .get(&description.surface.output)
                    .is_some_and(|desired| desired == description)
                    && self
                        .applied
                        .get(&description.surface.output)
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

fn valid_surface(surface: &rmac_top_bar::Surface) -> bool {
    !surface.output.0.is_empty()
        && surface.logical_width.is_finite()
        && surface.logical_width > 0.0
        && surface.logical_height == rmac_top_bar::BAR_HEIGHT
        && surface.scale.is_finite()
        && surface.scale > 0.0
        && surface.exclusive_zone == rmac_top_bar::BAR_HEIGHT
        && !surface.keyboard_interactive
}

fn valid_content(content: &rmac_top_bar::Content) -> bool {
    if content.system_mark.icon != rmac_top_bar::BuiltinIcon::System
        || content.system_mark.accessible != "rmac desktop"
        || !valid_text(&content.active_app)
        || content
            .workspace
            .as_deref()
            .is_some_and(|text| !valid_text(text))
        || !valid_text(&content.clock.visible)
        || !valid_text(&content.clock.accessible)
        || content.clock.activation != rmac_top_bar::PanelTarget::NotificationCenter
        || content.indicators.len() > 7
    {
        return false;
    }
    let mut kinds = Vec::new();
    content.indicators.iter().all(|indicator| {
        if kinds.contains(&indicator.kind) {
            return false;
        }
        kinds.push(indicator.kind);
        indicator.icon == rmac_top_bar::BuiltinIcon::from(indicator.kind)
            && indicator.activation == indicator.kind.panel_target()
            && valid_text(&indicator.visible)
            && valid_text(&indicator.accessible)
    })
}

fn valid_text(text: &str) -> bool {
    !text.trim().is_empty() && text.len() <= MAX_TEXT_BYTES && !text.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    fn projection(outputs: &[&str], minute: u32) -> rmac_top_bar::Projection {
        let mut status = rmac_shell_status::Snapshot {
            outputs: outputs
                .iter()
                .map(|output| rmac_shell_status::OutputContext {
                    id: (*output).into(),
                    logical_size: rmac_compositor::LogicalSize {
                        width: 1920.0,
                        height: 1080.0,
                    },
                    scale: 1.0,
                })
                .collect(),
            ..Default::default()
        };
        status.network = Some(rmac_shell_status::NetworkIndicator {
            state: rmac_shell_status::NetworkState::Connected,
            connection_name: None,
            wifi_strength: Some(80),
        });
        let now = chrono::FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 29, 12, minute, 0)
            .single()
            .unwrap();
        rmac_top_bar::project(&status, now, rmac_top_bar::LocaleHourCycle::TwentyFourHour)
    }

    fn apply_next(registry: &mut Registry) -> Command {
        let command = registry.next_command().unwrap().unwrap();
        registry.finish(command.id(), CommandResult::Applied);
        command
    }

    #[test]
    fn initial_surfaces_create_serially_in_stable_output_order() {
        let mut registry = Registry::default();
        registry
            .set_projection(&projection(&["B", "A"], 0))
            .unwrap();
        let first = apply_next(&mut registry);
        let second = apply_next(&mut registry);
        assert_eq!(first.kind.output().0, "A");
        assert_eq!(second.kind.output().0, "B");
        assert!(matches!(first.kind, CommandKind::Create { .. }));
        assert_eq!(registry.snapshot().applied.len(), 2);
        assert!(registry.next_command().unwrap().is_none());
    }

    #[test]
    fn content_changes_reconfigure_without_replacing_surface_identity() {
        let mut registry = Registry::default();
        registry.set_projection(&projection(&["A"], 0)).unwrap();
        let created = apply_next(&mut registry);
        registry.set_projection(&projection(&["A"], 1)).unwrap();
        let changed = registry.next_command().unwrap().unwrap();
        assert!(matches!(changed.kind, CommandKind::Reconfigure { .. }));
        assert_eq!(changed.kind.surface(), created.kind.surface());
        registry.finish(changed.id(), CommandResult::Applied);
        assert_eq!(
            registry.snapshot().applied[0]
                .description
                .content
                .clock
                .visible,
            "Wed Jul 29  12:01"
        );
    }

    #[test]
    fn hotplug_removes_before_creating_and_never_reuses_identity() {
        let mut registry = Registry::default();
        registry.set_projection(&projection(&["A"], 0)).unwrap();
        let original = apply_next(&mut registry);
        registry.set_projection(&projection(&["B"], 0)).unwrap();
        let remove = apply_next(&mut registry);
        assert!(matches!(remove.kind, CommandKind::Remove { .. }));
        let create = apply_next(&mut registry);
        assert_eq!(create.kind.output().0, "B");
        assert_ne!(create.kind.surface(), original.kind.surface());
    }

    #[test]
    fn superseded_and_stale_completions_converge_to_newest_content() {
        let mut registry = Registry::default();
        registry.set_projection(&projection(&["A"], 0)).unwrap();
        let create = registry.next_command().unwrap().unwrap();
        registry.set_projection(&projection(&["A"], 1)).unwrap();
        registry.finish(create.id(), CommandResult::Applied);
        let present = registry.next_command().unwrap().unwrap();
        assert!(matches!(present.kind, CommandKind::Reconfigure { .. }));
        registry.set_projection(&projection(&["A"], 2)).unwrap();
        registry.finish(present.id(), CommandResult::Failed);
        assert!(registry.snapshot().failures.is_empty());
        let newest = registry.next_command().unwrap().unwrap();
        assert_eq!(
            match &newest.kind {
                CommandKind::Reconfigure { description, .. } => {
                    description.content.clock.visible.as_str()
                }
                _ => panic!("new content should reconfigure"),
            },
            "Wed Jul 29  12:02"
        );
    }

    #[test]
    fn failure_is_output_scoped_and_exact_retry_restores_work() {
        let mut registry = Registry::default();
        registry
            .set_projection(&projection(&["A", "B"], 0))
            .unwrap();
        let first = registry.next_command().unwrap().unwrap();
        registry.finish(first.id(), CommandResult::Failed);
        assert_eq!(registry.snapshot().failures.len(), 1);
        let second = apply_next(&mut registry);
        assert_eq!(second.kind.output().0, "B");
        assert!(registry.next_command().unwrap().is_none());
        registry.retry(&"A".into());
        assert_eq!(
            registry.next_command().unwrap().unwrap().kind.output().0,
            "A"
        );
    }

    #[test]
    fn authoritative_close_recreates_and_makes_late_acknowledgement_inert() {
        let mut registry = Registry::default();
        registry.set_projection(&projection(&["A"], 0)).unwrap();
        let created = apply_next(&mut registry);
        registry.set_projection(&projection(&["A"], 1)).unwrap();
        let pending = registry.next_command().unwrap().unwrap();
        let closed = registry.surface_closed(created.kind.surface());
        assert!(closed.visible);
        let replacement = registry.next_command().unwrap().unwrap();
        assert!(matches!(replacement.kind, CommandKind::Create { .. }));
        assert_ne!(replacement.kind.surface(), created.kind.surface());
        assert!(
            !registry
                .finish(pending.id(), CommandResult::Applied)
                .visible
        );
    }

    #[test]
    fn invalid_projections_preserve_the_last_accepted_state() {
        let mut registry = Registry::default();
        let accepted = projection(&["A"], 0);
        registry.set_projection(&accepted).unwrap();
        let before = registry.snapshot();

        let mut duplicate = accepted.clone();
        duplicate.surfaces.push(duplicate.surfaces[0].clone());
        assert_eq!(
            registry.set_projection(&duplicate),
            Err(LifecycleError::DuplicateOutput)
        );
        let mut invalid_content = accepted;
        invalid_content.content.indicators[0].activation =
            rmac_top_bar::PanelTarget::NotificationCenter;
        assert_eq!(
            registry.set_projection(&invalid_content),
            Err(LifecycleError::InvalidContent)
        );
        assert_eq!(registry.snapshot(), before);
    }
}
