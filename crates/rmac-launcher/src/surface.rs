//! Exact placement and lifecycle for the centered launcher overlay.

use std::fmt;

pub const NAMESPACE: &str = "rmac-launcher";
/// The idle Spotlight surface contains the search capsule and four browse
/// mode controls used by macOS 26.
pub const LOGICAL_WIDTH: f64 = 644.0;
pub const LOGICAL_HEIGHT: f64 = 60.0;
/// Search results expand in place without opening a second surface.
pub const EXPANDED_LOGICAL_WIDTH: f64 = 720.0;
pub const EXPANDED_LOGICAL_HEIGHT: f64 = 540.0;
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
    Exclusive,
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
    pub margin_top: f64,
    pub margin_right: f64,
    pub margin_bottom: f64,
    pub margin_left: f64,
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
            .field("margin_top", &self.margin_top)
            .field("margin_right", &self.margin_right)
            .field("margin_bottom", &self.margin_bottom)
            .field("margin_left", &self.margin_left)
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
            Self::DoesNotFit => "the launcher does not fit on the invoking output",
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
    // The compact capsule must only open where its expanded results surface can
    // remain entirely on the invoking output.
    if logical.size.width < EXPANDED_LOGICAL_WIDTH || logical.size.height < EXPANDED_LOGICAL_HEIGHT
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
        anchor_top: false,
        anchor_right: false,
        anchor_bottom: false,
        anchor_left: false,
        logical_width: LOGICAL_WIDTH,
        logical_height: LOGICAL_HEIGHT,
        output_scale: logical.scale,
        margin_top: 0.0,
        margin_right: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        exclusive_zone: 0,
        keyboard_interactivity: KeyboardInteractivity::Exclusive,
    })
}

pub fn plan_invocation(
    invocation: &rmac_shell_invocation::Invocation,
    compositor: &rmac_compositor::Snapshot,
) -> Result<Description, PlanError> {
    let mut description = plan(
        invocation.output(),
        SeatId::new(invocation.seat().as_str())?,
        compositor,
    )?;
    description.invocation.restore_window = invocation
        .restore_window()
        .filter(|window| compositor.windows.iter().any(|item| item.id == *window));
    Ok(description)
}

#[derive(Clone, Eq, PartialEq)]
pub struct FocusRestoreRequest {
    pub seat: SeatId,
    pub window: rmac_compositor::WindowId,
}

impl fmt::Debug for FocusRestoreRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FocusRestoreRequest")
            .field("seat", &"<redacted>")
            .field("window", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FocusRestore {
    NotRequested,
    AlreadyFocused,
    TargetGone,
    Request(FocusRestoreRequest),
}

pub fn focus_restore(
    description: &Description,
    compositor: &rmac_compositor::Snapshot,
) -> FocusRestore {
    let Some(window) = description.invocation.restore_window else {
        return FocusRestore::NotRequested;
    };
    let Some(current) = compositor
        .windows
        .iter()
        .find(|candidate| candidate.id == window)
    else {
        return FocusRestore::TargetGone;
    };
    if current.focused && compositor.focus.window == Some(window) {
        FocusRestore::AlreadyFocused
    } else {
        FocusRestore::Request(FocusRestoreRequest {
            seat: description.invocation.seat.clone(),
            window,
        })
    }
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

    fn operation(&self) -> Operation {
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
            Self::InvalidPlan(error) => write!(formatter, "invalid launcher plan: {error}"),
            Self::InvalidDescription => {
                formatter.write_str("the launcher surface description is invalid")
            }
            Self::CommandExhausted => {
                formatter.write_str("launcher command identities are exhausted")
            }
            Self::SurfaceExhausted => {
                formatter.write_str("launcher surface identities are exhausted")
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
            Ok(description) if valid_description(description) => {
                let recovered = self.lifecycle_error.take().is_some();
                if self.desired.as_ref() == Some(description) {
                    recovered
                } else {
                    self.desired = Some(description.clone());
                    self.failure = None;
                    true
                }
            }
            Ok(_) => {
                let error = LifecycleError::InvalidDescription;
                let changed = self.lifecycle_error != Some(error);
                self.lifecycle_error = Some(error);
                changed
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
        && !description.anchor_top
        && !description.anchor_right
        && !description.anchor_bottom
        && !description.anchor_left
        && description.logical_width == LOGICAL_WIDTH
        && description.logical_height == LOGICAL_HEIGHT
        && description.output_scale.is_finite()
        && description.output_scale > 0.0
        && description.output_scale <= MAX_SCALE
        && description.margin_top == 0.0
        && description.margin_right == 0.0
        && description.margin_bottom == 0.0
        && description.margin_left == 0.0
        && description.exclusive_zone == 0
        && description.keyboard_interactivity == KeyboardInteractivity::Exclusive
        && !description.invocation.output.0.is_empty()
        && SeatId::new(description.invocation.seat.as_str()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(id: &str, width: f64, height: f64) -> rmac_compositor::Output {
        rmac_compositor::Output {
            id: id.into(),
            make: String::new(),
            model: String::new(),
            serial: None,
            physical_size_mm: None,
            modes: Vec::new(),
            current_mode: Some(0),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: Some(rmac_compositor::LogicalOutput {
                position: rmac_compositor::LogicalPoint::default(),
                size: rmac_compositor::LogicalSize { width, height },
                scale: 1.5,
                transform: "normal".into(),
            }),
        }
    }

    fn window(id: u64, focused: bool) -> rmac_compositor::Window {
        rmac_compositor::Window {
            id: rmac_compositor::WindowId(id),
            title: None,
            app_id: None,
            pid: None,
            workspace: None,
            focused,
            floating: false,
            urgent: false,
            focus_timestamp: None,
            layout: rmac_compositor::WindowLayout::default(),
        }
    }

    fn planned(output_id: &str, seat: &str) -> Result<Description, PlanError> {
        let snapshot = rmac_compositor::Snapshot {
            outputs: vec![output(output_id, 1920.0, 1080.0)],
            windows: vec![window(9, true)],
            focus: rmac_compositor::FocusState {
                output: Some(output_id.into()),
                window: Some(rmac_compositor::WindowId(9)),
                ..Default::default()
            },
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
    fn planner_is_centered_keyboard_owning_and_fails_closed() {
        let snapshot = rmac_compositor::Snapshot {
            outputs: vec![output("DP-2", 1920.0, 1080.0)],
            windows: vec![window(9, true)],
            focus: rmac_compositor::FocusState {
                output: Some("DP-2".into()),
                window: Some(rmac_compositor::WindowId(9)),
                ..Default::default()
            },
            ..Default::default()
        };
        let seats = rmac_shell_invocation::SeatInventory::new(vec!["seat-main".into()]).unwrap();
        let invocation = rmac_shell_invocation::global_shortcut(&snapshot, &seats).unwrap();
        let description = plan_invocation(&invocation, &snapshot).unwrap();
        assert_eq!(description.invocation.output.0, "DP-2");
        assert!(!description.anchor_top);
        assert!(!description.anchor_right);
        assert_eq!(
            description.keyboard_interactivity,
            KeyboardInteractivity::Exclusive
        );
        assert_eq!(
            description.invocation.restore_window,
            Some(rmac_compositor::WindowId(9))
        );

        let snapshot = rmac_compositor::Snapshot {
            outputs: vec![output("DP-2", 700.0, 540.0)],
            ..Default::default()
        };
        assert_eq!(
            plan(&"DP-2".into(), SeatId::new("seat-main").unwrap(), &snapshot),
            Err(PlanError::DoesNotFit)
        );
    }

    #[test]
    fn session_queues_transfers_and_acknowledges_serially() {
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
    fn host_loss_and_compositor_close_recreate_with_fresh_identity() {
        let mut session = ready_session();
        let create = session.open(&planned("A", "seat-a")).command.unwrap();
        let first_surface = create.kind.surface();
        session.finish(create.id(), CommandResult::Applied);
        session.host_stopped();
        let after_restart = session.host_ready().command.unwrap();
        assert_ne!(after_restart.kind.surface(), first_surface);
        let second_surface = after_restart.kind.surface();
        session.finish(after_restart.id(), CommandResult::Applied);
        let after_close = session.surface_closed(second_surface).command.unwrap();
        assert_ne!(after_close.kind.surface(), second_surface);
    }

    #[test]
    fn failure_retry_invalid_plan_and_stale_ack_are_explicit() {
        let mut session = ready_session();
        let create = session.open(&planned("A", "seat-a")).command.unwrap();
        session.finish(create.id(), CommandResult::Failed);
        assert!(session.snapshot().failure.is_some());
        let retried = session.retry().command.unwrap();
        session.host_stopped();
        assert!(!session.finish(retried.id(), CommandResult::Applied).changed);
        let rejected = session.open(&Err(PlanError::OutputMissing));
        assert_eq!(
            rejected.snapshot.lifecycle_error,
            Some(LifecycleError::InvalidPlan(PlanError::OutputMissing))
        );
        assert!(rejected.snapshot.desired.is_some());
        let mut forged = planned("B", "seat-b").unwrap();
        forged.anchor_top = true;
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
    }

    #[test]
    fn focus_restore_is_seat_bound_revalidated_and_private() {
        let description = planned("private-output-7", "private-seat-8").unwrap();
        let mut current = rmac_compositor::Snapshot {
            windows: vec![window(9, false)],
            ..Default::default()
        };
        let request = focus_restore(&description, &current);
        assert!(matches!(request, FocusRestore::Request(_)));
        let diagnostics = format!("{description:?} {request:?}");
        assert!(!diagnostics.contains("private-output-7"));
        assert!(!diagnostics.contains("private-seat-8"));
        current.windows.clear();
        assert_eq!(
            focus_restore(&description, &current),
            FocusRestore::TargetGone
        );
    }
}
