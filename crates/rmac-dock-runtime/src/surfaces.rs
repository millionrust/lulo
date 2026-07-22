//! Deterministic ownership and reconciliation for per-output Dock surfaces.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const MAX_DOCK_SURFACES: usize = 32;

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
pub enum CommandKind {
    Create {
        surface: SurfaceId,
        description: rmac_dock::SurfaceDescription,
    },
    Reconfigure {
        surface: SurfaceId,
        description: rmac_dock::SurfaceDescription,
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
                &description.output
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
    pub description: rmac_dock::SurfaceDescription,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LifecycleError {
    InvalidPlan(rmac_dock::motion::ConfigError),
    TooManySurfaces { count: usize },
    DuplicateOutput,
    CommandExhausted,
    SurfaceExhausted,
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPlan(error) => {
                write!(formatter, "the Dock surface plan is invalid: {error:?}")
            }
            Self::TooManySurfaces { count } => write!(
                formatter,
                "the Dock requested {count} surfaces; the limit is {MAX_DOCK_SURFACES}"
            ),
            Self::DuplicateOutput => {
                formatter.write_str("the Dock surface plan repeats an output identity")
            }
            Self::CommandExhausted => {
                formatter.write_str("the Dock surface command identity space is exhausted")
            }
            Self::SurfaceExhausted => {
                formatter.write_str("the Dock surface identity space is exhausted")
            }
        }
    }
}

impl std::error::Error for LifecycleError {}

#[derive(Clone, Debug)]
struct Applied {
    surface: SurfaceId,
    description: rmac_dock::SurfaceDescription,
}

#[derive(Default)]
pub struct Registry {
    desired: BTreeMap<rmac_compositor::OutputId, rmac_dock::SurfaceDescription>,
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

    /// Accept the newest complete runtime plan. An invalid or oversized plan
    /// leaves the last accepted desired state untouched.
    pub fn set_runtime_snapshot(
        &mut self,
        snapshot: &crate::Snapshot,
    ) -> Result<bool, LifecycleError> {
        self.set_desired(&snapshot.surface_plan)
    }

    pub fn set_desired(
        &mut self,
        plan: &Result<Vec<rmac_dock::SurfaceDescription>, rmac_dock::motion::ConfigError>,
    ) -> Result<bool, LifecycleError> {
        let descriptions = plan
            .as_ref()
            .map_err(|error| LifecycleError::InvalidPlan(*error))?;
        if descriptions.len() > MAX_DOCK_SURFACES {
            return Err(LifecycleError::TooManySurfaces {
                count: descriptions.len(),
            });
        }
        let mut desired = BTreeMap::new();
        for description in descriptions {
            if desired
                .insert(description.output.clone(), description.clone())
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

    /// Yield at most one command. The renderer must acknowledge it before the
    /// registry issues another, so applied state always names real surfaces.
    pub fn next_command(&mut self) -> Result<Option<Command>, LifecycleError> {
        if self.pending.is_some() {
            return Ok(None);
        }
        let kind = self.next_kind()?;
        let Some(kind) = kind else {
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
            CommandResult::Failed => {
                self.failures.insert(output, command.kind.operation());
            }
        }
        Transition {
            snapshot: self.snapshot(),
            visible: true,
        }
    }

    /// Cancel work that is known not to have touched the platform. If the
    /// renderer cannot prove that, it must report Failed and reconcile through
    /// explicit retry instead of guessing the applied state.
    pub fn cancel(&mut self, id: CommandId) -> Transition {
        if self
            .pending
            .as_ref()
            .is_some_and(|command| command.id == id)
        {
            self.pending = None;
            Transition {
                snapshot: self.snapshot(),
                visible: true,
            }
        } else {
            Transition {
                snapshot: self.snapshot(),
                visible: false,
            }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn description(
        output: &str,
        placement: rmac_shell_settings::DockPlacement,
    ) -> rmac_dock::SurfaceDescription {
        rmac_dock::SurfaceDescription {
            output: rmac_compositor::OutputId::from(output),
            placement,
            output_axis_length: 1920.0,
            output_scale: 1.0,
            base_thickness: 64.0,
            maximum_thickness: 88.0,
            exclusive_zone: 64.0,
            reveal_edge_thickness: 0.0,
            keyboard_interactive: false,
            autohide: false,
            overview_visible: false,
            magnification_enabled: true,
            animate: true,
            magnification: rmac_dock::motion::MagnificationConfig::default(),
        }
    }

    fn apply_next(registry: &mut Registry) -> Command {
        let command = registry.next_command().unwrap().unwrap();
        registry.finish(command.id(), CommandResult::Applied);
        command
    }

    #[test]
    fn initial_surfaces_are_created_in_stable_output_order() {
        let mut registry = Registry::default();
        registry
            .set_desired(&Ok(vec![
                description("B", rmac_shell_settings::DockPlacement::Bottom),
                description("A", rmac_shell_settings::DockPlacement::Bottom),
            ]))
            .unwrap();

        let first = registry.next_command().unwrap().unwrap();
        assert_eq!(first.kind.output(), &rmac_compositor::OutputId::from("A"));
        assert!(matches!(&first.kind, CommandKind::Create { .. }));
        assert!(registry.next_command().unwrap().is_none());
        registry.finish(first.id(), CommandResult::Applied);
        let second = apply_next(&mut registry);
        assert_eq!(second.kind.output(), &rmac_compositor::OutputId::from("B"));
        assert!(registry.next_command().unwrap().is_none());
        assert_eq!(registry.snapshot().applied.len(), 2);
    }

    #[test]
    fn hotplug_removes_old_surface_before_creating_the_new_one() {
        let mut registry = Registry::default();
        registry
            .set_desired(&Ok(vec![description(
                "A",
                rmac_shell_settings::DockPlacement::Bottom,
            )]))
            .unwrap();
        apply_next(&mut registry);
        registry
            .set_desired(&Ok(vec![description(
                "B",
                rmac_shell_settings::DockPlacement::Bottom,
            )]))
            .unwrap();

        let remove = apply_next(&mut registry);
        assert!(matches!(&remove.kind, CommandKind::Remove { .. }));
        assert_eq!(remove.kind.output(), &rmac_compositor::OutputId::from("A"));
        let create = apply_next(&mut registry);
        assert!(matches!(&create.kind, CommandKind::Create { .. }));
        assert_eq!(create.kind.output(), &rmac_compositor::OutputId::from("B"));
    }

    #[test]
    fn policy_changes_reconfigure_the_existing_surface_identity() {
        let mut registry = Registry::default();
        registry
            .set_desired(&Ok(vec![description(
                "A",
                rmac_shell_settings::DockPlacement::Bottom,
            )]))
            .unwrap();
        let created = apply_next(&mut registry);
        let surface = created.kind.surface();
        registry
            .set_desired(&Ok(vec![description(
                "A",
                rmac_shell_settings::DockPlacement::Left,
            )]))
            .unwrap();

        let updated = apply_next(&mut registry);
        assert!(matches!(&updated.kind, CommandKind::Reconfigure { .. }));
        assert_eq!(updated.kind.surface(), surface);
        assert_eq!(
            registry.snapshot().applied[0].description.placement,
            rmac_shell_settings::DockPlacement::Left
        );
    }

    #[test]
    fn a_new_desired_plan_supersedes_pending_work_after_acknowledgement() {
        let mut registry = Registry::default();
        registry
            .set_desired(&Ok(vec![description(
                "A",
                rmac_shell_settings::DockPlacement::Bottom,
            )]))
            .unwrap();
        let create = registry.next_command().unwrap().unwrap();
        registry
            .set_desired(&Ok(vec![description(
                "A",
                rmac_shell_settings::DockPlacement::Right,
            )]))
            .unwrap();
        registry.finish(create.id(), CommandResult::Applied);

        let update = apply_next(&mut registry);
        assert!(matches!(&update.kind, CommandKind::Reconfigure { .. }));
        assert_eq!(update.kind.surface(), create.kind.surface());
        assert_eq!(
            registry.snapshot().applied[0].description.placement,
            rmac_shell_settings::DockPlacement::Right
        );
    }

    #[test]
    fn failure_blocks_only_that_output_until_explicit_retry() {
        let mut registry = Registry::default();
        registry
            .set_desired(&Ok(vec![
                description("A", rmac_shell_settings::DockPlacement::Bottom),
                description("B", rmac_shell_settings::DockPlacement::Bottom),
            ]))
            .unwrap();
        let failed = registry.next_command().unwrap().unwrap();
        let transition = registry.finish(failed.id(), CommandResult::Failed);
        assert_eq!(transition.snapshot.failures.len(), 1);

        let other = apply_next(&mut registry);
        assert_eq!(other.kind.output(), &rmac_compositor::OutputId::from("B"));
        assert!(registry.next_command().unwrap().is_none());
        assert!(
            registry
                .retry(&rmac_compositor::OutputId::from("A"))
                .visible
        );
        let retried = apply_next(&mut registry);
        assert_eq!(retried.kind.output(), &rmac_compositor::OutputId::from("A"));
    }

    #[test]
    fn invalid_duplicate_and_oversized_plans_preserve_last_desired_state() {
        let mut registry = Registry::default();
        let accepted = description("A", rmac_shell_settings::DockPlacement::Bottom);
        registry.set_desired(&Ok(vec![accepted.clone()])).unwrap();
        assert_eq!(
            registry.set_desired(&Err(rmac_dock::motion::ConfigError::MaximumScale)),
            Err(LifecycleError::InvalidPlan(
                rmac_dock::motion::ConfigError::MaximumScale
            ))
        );
        assert_eq!(
            registry.set_desired(&Ok(vec![accepted.clone(), accepted])),
            Err(LifecycleError::DuplicateOutput)
        );
        let too_many = (0..=MAX_DOCK_SURFACES)
            .map(|index| {
                description(
                    &format!("output-{index}"),
                    rmac_shell_settings::DockPlacement::Bottom,
                )
            })
            .collect();
        assert_eq!(
            registry.set_desired(&Ok(too_many)),
            Err(LifecycleError::TooManySurfaces {
                count: MAX_DOCK_SURFACES + 1
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
            .set_desired(&Ok(vec![description(
                "A",
                rmac_shell_settings::DockPlacement::Bottom,
            )]))
            .unwrap();
        let cancelled = registry.next_command().unwrap().unwrap();
        assert!(registry.cancel(cancelled.id()).visible);
        let replacement = registry.next_command().unwrap().unwrap();
        let stale = registry.finish(cancelled.id(), CommandResult::Applied);
        assert!(!stale.visible);
        assert!(stale.snapshot.applied.is_empty());
        registry.finish(replacement.id(), CommandResult::Applied);
        assert_eq!(registry.snapshot().applied.len(), 1);
    }
}
