//! Exact placement and lifecycle for the on-demand Notification Center panel.

use std::fmt;

pub const NAMESPACE: &str = "rmac-notification-center";
pub const LOGICAL_WIDTH: f64 = 420.0;
pub const LOGICAL_HEIGHT: f64 = 720.0;
pub const TOP_MARGIN: f64 = 44.0;
pub const RIGHT_MARGIN: f64 = 12.0;
pub const MAX_SEAT_ID_BYTES: usize = 128;
const MAX_SCALE: f64 = 8.0;

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct SeatId(String);

impl SeatId {
    pub fn new(value: impl Into<String>) -> Result<Self, PlanError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_SEAT_ID_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(PlanError::InvalidSeat);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SeatId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SeatId(<redacted>)")
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Invocation {
    pub output: rmac_compositor::OutputId,
    pub seat: SeatId,
    pub restore_window: Option<rmac_compositor::WindowId>,
}

impl fmt::Debug for Invocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Invocation")
            .field("output", &"<redacted>")
            .field("seat", &"<redacted>")
            .field("restore_window", &self.restore_window.is_some())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Layer {
    Overlay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyboardInteractivity {
    OnDemand,
}

#[derive(Clone, PartialEq)]
pub struct Description {
    pub invocation: Invocation,
    pub namespace: &'static str,
    pub layer: Layer,
    pub anchor_top: bool,
    pub anchor_right: bool,
    pub anchor_bottom: bool,
    pub anchor_left: bool,
    pub logical_width: f64,
    pub logical_height: f64,
    pub output_scale: f64,
    pub top_margin: f64,
    pub right_margin: f64,
    pub exclusive_zone: i32,
    pub keyboard_interactivity: KeyboardInteractivity,
}

impl fmt::Debug for Description {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Description")
            .field("invocation", &self.invocation)
            .field("namespace", &self.namespace)
            .field("layer", &self.layer)
            .field("anchor_top", &self.anchor_top)
            .field("anchor_right", &self.anchor_right)
            .field("anchor_bottom", &self.anchor_bottom)
            .field("anchor_left", &self.anchor_left)
            .field("logical_width", &self.logical_width)
            .field("logical_height", &self.logical_height)
            .field("output_scale", &self.output_scale)
            .field("top_margin", &self.top_margin)
            .field("right_margin", &self.right_margin)
            .field("exclusive_zone", &self.exclusive_zone)
            .field("keyboard_interactivity", &self.keyboard_interactivity)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanError {
    InvalidSeat,
    OutputMissing,
    OutputDisabled,
    InvalidGeometry,
    DoesNotFit,
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSeat => "the invoking seat identity is invalid",
            Self::OutputMissing => "the invoking output is no longer available",
            Self::OutputDisabled => "the invoking output is disabled",
            Self::InvalidGeometry => "the invoking output geometry is invalid",
            Self::DoesNotFit => "Notification Center does not fit on the invoking output",
        })
    }
}

impl std::error::Error for PlanError {}

pub fn plan(
    output_id: &rmac_compositor::OutputId,
    seat: SeatId,
    compositor: &rmac_compositor::Snapshot,
) -> Result<Description, PlanError> {
    let output = compositor
        .outputs
        .iter()
        .find(|output| &output.id == output_id)
        .ok_or(PlanError::OutputMissing)?;
    if !output.enabled() {
        return Err(PlanError::OutputDisabled);
    }
    let logical = output.logical.as_ref().ok_or(PlanError::OutputDisabled)?;
    if !logical.size.is_valid()
        || logical.size.width <= 0.0
        || logical.size.height <= 0.0
        || !logical.scale.is_finite()
        || logical.scale <= 0.0
        || logical.scale > MAX_SCALE
    {
        return Err(PlanError::InvalidGeometry);
    }
    if logical.size.width < LOGICAL_WIDTH + RIGHT_MARGIN
        || logical.size.height < LOGICAL_HEIGHT + TOP_MARGIN
    {
        return Err(PlanError::DoesNotFit);
    }
    let restore_window = compositor.focus.window.filter(|focused| {
        compositor
            .windows
            .iter()
            .any(|window| window.id == *focused && window.focused)
    });
    Ok(Description {
        invocation: Invocation {
            output: output_id.clone(),
            seat,
            restore_window,
        },
        namespace: NAMESPACE,
        layer: Layer::Overlay,
        anchor_top: true,
        anchor_right: true,
        anchor_bottom: false,
        anchor_left: false,
        logical_width: LOGICAL_WIDTH,
        logical_height: LOGICAL_HEIGHT,
        output_scale: logical.scale,
        top_margin: TOP_MARGIN,
        right_margin: RIGHT_MARGIN,
        exclusive_zone: 0,
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
    })
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CommandId(u64);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SurfaceId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Create,
    Reconfigure,
    Remove,
}

#[derive(Clone, PartialEq)]
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
                &description.invocation.output
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

    pub fn operation(&self) -> Operation {
        match self {
            Self::Create { .. } => Operation::Create,
            Self::Reconfigure { .. } => Operation::Reconfigure,
            Self::Remove { .. } => Operation::Remove,
        }
    }
}

impl fmt::Debug for CommandKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandKind")
            .field("operation", &self.operation())
            .field("surface", &self.surface())
            .field("output", &"<redacted>")
            .finish()
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

#[derive(Clone, PartialEq)]
pub struct Applied {
    pub surface: SurfaceId,
    pub description: Description,
}

impl fmt::Debug for Applied {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Applied")
            .field("surface", &self.surface)
            .field("description", &self.description)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub operation: Operation,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub desired: Option<Description>,
    pub applied: Option<Applied>,
    pub pending: Option<Command>,
    pub failure: Option<Failure>,
    pub lifecycle_error: Option<LifecycleError>,
    pub host_ready: bool,
}

#[derive(Clone, Debug)]
pub struct Transition {
    pub snapshot: Snapshot,
    pub command: Option<Command>,
    pub changed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleError {
    InvalidPlan(PlanError),
    InvalidDescription,
    CommandExhausted,
    SurfaceExhausted,
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPlan(error) => {
                write!(formatter, "invalid Notification Center plan: {error}")
            }
            Self::InvalidDescription => {
                formatter.write_str("the Notification Center surface description is invalid")
            }
            Self::CommandExhausted => {
                formatter.write_str("Notification Center command identities are exhausted")
            }
            Self::SurfaceExhausted => {
                formatter.write_str("Notification Center surface identities are exhausted")
            }
        }
    }
}

impl std::error::Error for LifecycleError {}

#[derive(Default)]
pub struct Session {
    desired: Option<Description>,
    applied: Option<Applied>,
    pending: Option<Command>,
    failure: Option<Failure>,
    lifecycle_error: Option<LifecycleError>,
    host_ready: bool,
    next_command: u64,
    next_surface: u64,
}

impl Session {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            desired: self.desired.clone(),
            applied: self.applied.clone(),
            pending: self.pending.clone(),
            failure: self.failure.clone(),
            lifecycle_error: self.lifecycle_error,
            host_ready: self.host_ready,
        }
    }

    pub fn open(&mut self, plan: &Result<Description, PlanError>) -> Transition {
        let changed = match plan {
            Ok(description) => {
                if !valid_description(description) {
                    let error = LifecycleError::InvalidDescription;
                    let changed = self.lifecycle_error != Some(error);
                    self.lifecycle_error = Some(error);
                    return self.issue_next(changed);
                }
                let recovered = self.lifecycle_error.take().is_some();
                if self.desired.as_ref() == Some(description) {
                    recovered
                } else {
                    self.desired = Some(description.clone());
                    self.failure = None;
                    true
                }
            }
            Err(error) => {
                let error = LifecycleError::InvalidPlan(*error);
                let changed = self.lifecycle_error != Some(error);
                self.lifecycle_error = Some(error);
                changed
            }
        };
        self.issue_next(changed)
    }

    pub fn close(&mut self) -> Transition {
        let desired = self.desired.take().is_some();
        let failure = self.failure.take().is_some();
        let recovered = self.lifecycle_error.take().is_some();
        self.issue_next(desired || failure || recovered)
    }

    pub fn host_ready(&mut self) -> Transition {
        let changed = !self.host_ready;
        self.host_ready = true;
        self.issue_next(changed)
    }

    pub fn host_stopped(&mut self) -> Transition {
        let mut changed = self.host_ready;
        self.host_ready = false;
        changed |= self.pending.take().is_some();
        changed |= self.applied.take().is_some();
        changed |= self.failure.take().is_some();
        Transition {
            snapshot: self.snapshot(),
            command: None,
            changed,
        }
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
                command: None,
                changed: false,
            };
        };
        self.pending = None;
        match result {
            CommandResult::Applied => self.apply_command(&command),
            CommandResult::Failed => {
                if self.failure_is_relevant(&command.kind) {
                    self.failure = Some(Failure {
                        operation: command.kind.operation(),
                    });
                }
            }
        }
        self.issue_next(true)
    }

    pub fn retry(&mut self) -> Transition {
        let changed = self.failure.take().is_some();
        self.issue_next(changed)
    }

    pub fn surface_closed(&mut self, surface: SurfaceId) -> Transition {
        let pending = self
            .pending
            .as_ref()
            .is_some_and(|command| command.kind.surface() == surface);
        if pending {
            self.pending = None;
        }
        let applied = self
            .applied
            .as_ref()
            .is_some_and(|applied| applied.surface == surface);
        if applied {
            self.applied = None;
        }
        let failure = if pending || applied {
            self.failure.take().is_some()
        } else {
            false
        };
        self.issue_next(pending || applied || failure)
    }

    fn issue_next(&mut self, mut changed: bool) -> Transition {
        if !self.host_ready || self.pending.is_some() || self.failure.is_some() {
            return Transition {
                snapshot: self.snapshot(),
                command: None,
                changed,
            };
        }
        let command = match self.next_kind() {
            Ok(Some(kind)) => match self.next_command.checked_add(1) {
                Some(id) => {
                    self.next_command = id;
                    Some(Command {
                        id: CommandId(id),
                        kind,
                    })
                }
                None => {
                    let error = LifecycleError::CommandExhausted;
                    changed |= self.lifecycle_error != Some(error);
                    self.lifecycle_error = Some(error);
                    None
                }
            },
            Ok(None) => None,
            Err(error) => {
                changed |= self.lifecycle_error != Some(error);
                self.lifecycle_error = Some(error);
                None
            }
        };
        if let Some(command) = command.as_ref() {
            self.pending = Some(command.clone());
            changed = true;
        }
        Transition {
            snapshot: self.snapshot(),
            command,
            changed,
        }
    }

    fn next_kind(&mut self) -> Result<Option<CommandKind>, LifecycleError> {
        match (&self.applied, &self.desired) {
            (Some(applied), Some(desired))
                if applied.description.invocation.output != desired.invocation.output =>
            {
                Ok(Some(CommandKind::Remove {
                    surface: applied.surface,
                    output: applied.description.invocation.output.clone(),
                }))
            }
            (Some(applied), Some(desired)) if &applied.description != desired => {
                Ok(Some(CommandKind::Reconfigure {
                    surface: applied.surface,
                    description: desired.clone(),
                }))
            }
            (Some(applied), None) => Ok(Some(CommandKind::Remove {
                surface: applied.surface,
                output: applied.description.invocation.output.clone(),
            })),
            (None, Some(desired)) => {
                let id = self
                    .next_surface
                    .checked_add(1)
                    .ok_or(LifecycleError::SurfaceExhausted)?;
                self.next_surface = id;
                Ok(Some(CommandKind::Create {
                    surface: SurfaceId(id),
                    description: desired.clone(),
                }))
            }
            _ => Ok(None),
        }
    }

    fn apply_command(&mut self, command: &Command) {
        match &command.kind {
            CommandKind::Create {
                surface,
                description,
            }
            | CommandKind::Reconfigure {
                surface,
                description,
            } => {
                self.applied = Some(Applied {
                    surface: *surface,
                    description: description.clone(),
                });
            }
            CommandKind::Remove { surface, .. } => {
                if self
                    .applied
                    .as_ref()
                    .is_some_and(|applied| applied.surface == *surface)
                {
                    self.applied = None;
                }
            }
        }
        self.failure = None;
    }

    fn failure_is_relevant(&self, kind: &CommandKind) -> bool {
        match kind {
            CommandKind::Create { description, .. }
            | CommandKind::Reconfigure { description, .. } => {
                self.desired.as_ref() == Some(description)
            }
            CommandKind::Remove { surface, .. } => {
                self.applied
                    .as_ref()
                    .is_some_and(|applied| applied.surface == *surface)
                    && self.desired.as_ref() != self.applied.as_ref().map(|item| &item.description)
            }
        }
    }
}

fn valid_description(description: &Description) -> bool {
    description.namespace == NAMESPACE
        && description.layer == Layer::Overlay
        && description.anchor_top
        && description.anchor_right
        && !description.anchor_bottom
        && !description.anchor_left
        && description.logical_width == LOGICAL_WIDTH
        && description.logical_height == LOGICAL_HEIGHT
        && description.output_scale.is_finite()
        && description.output_scale > 0.0
        && description.output_scale <= MAX_SCALE
        && description.top_margin == TOP_MARGIN
        && description.right_margin == RIGHT_MARGIN
        && description.exclusive_zone == 0
        && description.keyboard_interactivity == KeyboardInteractivity::OnDemand
        && !description.invocation.output.0.is_empty()
        && SeatId::new(description.invocation.seat.as_str()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(id: &str, enabled: bool, width: f64, height: f64) -> rmac_compositor::Output {
        rmac_compositor::Output {
            id: id.into(),
            make: String::new(),
            model: String::new(),
            serial: None,
            physical_size_mm: None,
            modes: Vec::new(),
            current_mode: enabled.then_some(0),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: enabled.then_some(rmac_compositor::LogicalOutput {
                position: rmac_compositor::LogicalPoint::default(),
                size: rmac_compositor::LogicalSize { width, height },
                scale: 1.5,
                transform: "normal".into(),
            }),
        }
    }

    fn planned(output_id: &str, seat: &str) -> Result<Description, PlanError> {
        let snapshot = rmac_compositor::Snapshot {
            outputs: vec![output(output_id, true, 1920.0, 1080.0)],
            ..Default::default()
        };
        plan(&output_id.into(), SeatId::new(seat).unwrap(), &snapshot)
    }

    fn ready_session() -> Session {
        let mut session = Session::default();
        assert!(session.host_ready().changed);
        session
    }

    #[test]
    fn planner_targets_exact_output_and_fails_closed() {
        let description = planned("DP-2", "seat-main").unwrap();
        assert_eq!(description.invocation.output.0, "DP-2");
        assert_eq!(description.output_scale, 1.5);
        assert_eq!(description.logical_width, LOGICAL_WIDTH);
        assert_eq!(description.logical_height, LOGICAL_HEIGHT);

        let snapshot = rmac_compositor::Snapshot {
            outputs: vec![output("DP-2", true, 420.0, 740.0)],
            ..Default::default()
        };
        assert_eq!(
            plan(&"DP-2".into(), SeatId::new("seat-main").unwrap(), &snapshot),
            Err(PlanError::DoesNotFit)
        );
    }

    #[test]
    fn desired_panel_waits_for_host_and_transfers_serially() {
        let mut session = Session::default();
        assert!(session.open(&planned("A", "seat-a")).command.is_none());
        let create = session.host_ready().command.unwrap();
        session.open(&planned("B", "seat-b"));
        let remove = session
            .finish(create.id(), CommandResult::Applied)
            .command
            .unwrap();
        assert!(matches!(remove.kind, CommandKind::Remove { .. }));
        let replacement = session
            .finish(remove.id(), CommandResult::Applied)
            .command
            .unwrap();
        assert_eq!(replacement.kind.output().0, "B");
    }

    #[test]
    fn host_loss_recreates_desired_panel_with_fresh_identity() {
        let mut session = ready_session();
        let create = session.open(&planned("A", "seat-a")).command.unwrap();
        let old_surface = create.kind.surface();
        session.finish(create.id(), CommandResult::Applied);
        let pending = session.open(&planned("A", "seat-b")).command.unwrap();
        let stopped = session.host_stopped();
        assert!(stopped.snapshot.applied.is_none());
        assert!(stopped.snapshot.desired.is_some());
        assert!(!stopped.snapshot.host_ready);
        assert!(!session.finish(pending.id(), CommandResult::Applied).changed);
        let replacement = session.host_ready().command.unwrap();
        assert_ne!(replacement.kind.surface(), old_surface);
    }

    #[test]
    fn compositor_close_and_failure_retry_converge() {
        let mut session = ready_session();
        let create = session.open(&planned("A", "seat-a")).command.unwrap();
        let old_surface = create.kind.surface();
        session.finish(create.id(), CommandResult::Applied);
        let replacement = session.surface_closed(old_surface).command.unwrap();
        assert_ne!(replacement.kind.surface(), old_surface);
        session.finish(replacement.id(), CommandResult::Failed);
        assert!(session.snapshot().failure.is_some());
        assert!(session.retry().command.is_some());
    }

    #[test]
    fn invalid_plan_preserves_state_until_close_or_valid_recovery() {
        let mut session = ready_session();
        let create = session.open(&planned("A", "seat-a")).command.unwrap();
        session.finish(create.id(), CommandResult::Applied);
        let rejected = session.open(&Err(PlanError::OutputMissing));
        assert!(rejected.snapshot.applied.is_some());
        assert_eq!(
            rejected.snapshot.lifecycle_error,
            Some(LifecycleError::InvalidPlan(PlanError::OutputMissing))
        );
        let mut forged = planned("B", "seat-b").unwrap();
        forged.anchor_left = true;
        assert_eq!(
            session.open(&Ok(forged)).snapshot.lifecycle_error,
            Some(LifecycleError::InvalidDescription)
        );
        assert_eq!(
            session
                .snapshot()
                .desired
                .as_ref()
                .unwrap()
                .invocation
                .output
                .0,
            "A"
        );
        assert!(session
            .open(&planned("A", "seat-a"))
            .snapshot
            .lifecycle_error
            .is_none());
        assert!(session.close().command.is_some());
    }

    #[test]
    fn diagnostics_redact_output_and_seat_identity() {
        let mut session = Session::default();
        let diagnostics = format!(
            "{:?}",
            session
                .open(&planned("private-output-94", "private-seat-73"))
                .snapshot
        );
        assert!(!diagnostics.contains("private-output-94"));
        assert!(!diagnostics.contains("private-seat-73"));
    }
}
