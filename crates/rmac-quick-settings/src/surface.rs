//! Exact invocation, placement, and ownership for the Quick Settings surface.

use std::fmt;

pub const NAMESPACE: &str = "rmac-quick-settings";
pub const LOGICAL_WIDTH: f64 = crate::layout::SURFACE_WIDTH;
/// The surface opens at the height of its always-present modules and is then
/// resized to fit what it shows (see [`crate::layout::Modules`]).
pub const LOGICAL_HEIGHT: f64 = crate::layout::MIN_SURFACE_HEIGHT;
/// Measured on macOS 26: the first module row starts 40 below the menu bar
/// and the modules end 14 from the screen edge; the surface padding sits
/// outside them.
pub const TOP_MARGIN: f64 = 40.0 - crate::layout::PADDING;
pub const RIGHT_MARGIN: f64 = 14.0 - crate::layout::PADDING;
pub const MAX_SEAT_ID_BYTES: usize = 128;
const MAX_SCALE: f64 = 8.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Layer {
    Overlay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Anchors {
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
    pub left: bool,
}

impl Anchors {
    pub const TOP_RIGHT: Self = Self {
        top: true,
        right: true,
        bottom: false,
        left: false,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyboardInteractivity {
    OnDemand,
}

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

#[derive(Clone, PartialEq)]
pub struct Description {
    pub invocation: Invocation,
    pub namespace: &'static str,
    pub layer: Layer,
    pub anchors: Anchors,
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
            .field("anchors", &self.anchors)
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
            Self::DoesNotFit => "Quick Settings does not fit on the invoking output",
        })
    }
}

impl std::error::Error for PlanError {}

/// Capture one invocation against the same complete compositor snapshot used
/// to validate its exact output. A dangling focused window is never retained.
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
        || !(0.0..=MAX_SCALE).contains(&logical.scale)
        || logical.scale == 0.0
    {
        return Err(PlanError::InvalidGeometry);
    }
    if logical.size.width < LOGICAL_WIDTH + RIGHT_MARGIN
        || logical.size.height < crate::layout::MAX_SURFACE_HEIGHT + TOP_MARGIN
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
        anchors: Anchors::TOP_RIGHT,
        logical_width: LOGICAL_WIDTH,
        logical_height: LOGICAL_HEIGHT,
        output_scale: logical.scale,
        top_margin: TOP_MARGIN,
        right_margin: RIGHT_MARGIN,
        exclusive_zone: 0,
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
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

/// Revalidate the invocation's captured window immediately before focus
/// restoration. The platform adapter must honor the exact seat or report that
/// seat-scoped restoration is unsupported; it must not silently focus globally.
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
pub enum SurfaceOperation {
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

    pub fn operation(&self) -> SurfaceOperation {
        match self {
            Self::Create { .. } => SurfaceOperation::Create,
            Self::Reconfigure { .. } => SurfaceOperation::Reconfigure,
            Self::Remove { .. } => SurfaceOperation::Remove,
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
pub struct AppliedSurface {
    pub surface: SurfaceId,
    pub description: Description,
}

impl fmt::Debug for AppliedSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppliedSurface")
            .field("surface", &self.surface)
            .field("description", &self.description)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct SurfaceFailure {
    pub output: rmac_compositor::OutputId,
    pub operation: SurfaceOperation,
}

impl fmt::Debug for SurfaceFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SurfaceFailure")
            .field("output", &"<redacted>")
            .field("operation", &self.operation)
            .finish()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub desired: Option<Description>,
    pub applied: Option<AppliedSurface>,
    pub pending: Option<Command>,
    pub failure: Option<SurfaceFailure>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transition {
    pub snapshot: Snapshot,
    pub visible: bool,
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
            Self::InvalidPlan(error) => write!(formatter, "invalid Quick Settings plan: {error}"),
            Self::InvalidDescription => {
                formatter.write_str("the Quick Settings surface description is invalid")
            }
            Self::CommandExhausted => {
                formatter.write_str("Quick Settings command identities are exhausted")
            }
            Self::SurfaceExhausted => {
                formatter.write_str("Quick Settings surface identities are exhausted")
            }
        }
    }
}

impl std::error::Error for LifecycleError {}

#[derive(Clone)]
struct Applied {
    surface: SurfaceId,
    description: Description,
}

#[derive(Default)]
pub struct Registry {
    desired: Option<Description>,
    applied: Option<Applied>,
    pending: Option<Command>,
    failure: Option<SurfaceFailure>,
    next_command: u64,
    next_surface: u64,
}

impl Registry {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            desired: self.desired.clone(),
            applied: self.applied.as_ref().map(|applied| AppliedSurface {
                surface: applied.surface,
                description: applied.description.clone(),
            }),
            pending: self.pending.clone(),
            failure: self.failure.clone(),
        }
    }

    /// Accept a complete invocation plan. Invalid input preserves the previous
    /// desired and applied state.
    pub fn open(&mut self, plan: &Result<Description, PlanError>) -> Result<bool, LifecycleError> {
        let description = plan
            .as_ref()
            .map_err(|error| LifecycleError::InvalidPlan(*error))?;
        if !valid_description(description) {
            return Err(LifecycleError::InvalidDescription);
        }
        if self.desired.as_ref() == Some(description) {
            return Ok(false);
        }
        self.desired = Some(description.clone());
        self.failure = None;
        Ok(true)
    }

    pub fn close(&mut self) -> bool {
        let desired = self.desired.take().is_some();
        let failure = self.failure.take().is_some();
        desired || failure
    }

    pub fn next_command(&mut self) -> Result<Option<Command>, LifecycleError> {
        if self.pending.is_some() || self.failure.is_some() {
            return Ok(None);
        }
        let Some(kind) = self.next_kind()? else {
            return Ok(None);
        };
        let id = self
            .next_command
            .checked_add(1)
            .ok_or(LifecycleError::CommandExhausted)?;
        self.next_command = id;
        let command = Command {
            id: CommandId(id),
            kind,
        };
        self.pending = Some(command.clone());
        Ok(Some(command))
    }

    pub fn finish(&mut self, id: CommandId, result: CommandResult) -> Transition {
        let before = self.snapshot();
        let Some(command) = self
            .pending
            .as_ref()
            .filter(|command| command.id == id)
            .cloned()
        else {
            return Transition {
                snapshot: before,
                visible: false,
            };
        };
        self.pending = None;
        match result {
            CommandResult::Applied => self.apply_command(&command),
            CommandResult::Failed => {
                if self.failure_is_relevant(&command) {
                    self.failure = Some(SurfaceFailure {
                        output: command.kind.output().clone(),
                        operation: command.kind.operation(),
                    });
                }
            }
        }
        let snapshot = self.snapshot();
        Transition {
            visible: snapshot != before,
            snapshot,
        }
    }

    pub fn retry(&mut self) -> Transition {
        let visible = self.failure.take().is_some();
        Transition {
            snapshot: self.snapshot(),
            visible,
        }
    }

    pub fn surface_closed(&mut self, surface: SurfaceId) -> Transition {
        let before = self.snapshot();
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
            .as_ref()
            .filter(|applied| applied.surface == surface)
            .map(|applied| applied.description.invocation.output.clone());
        if applied_output.is_some() {
            self.applied = None;
        }
        if self.failure.as_ref().is_some_and(|failure| {
            pending_output.as_ref() == Some(&failure.output)
                || applied_output.as_ref() == Some(&failure.output)
        }) {
            self.failure = None;
        }
        let snapshot = self.snapshot();
        Transition {
            visible: snapshot != before,
            snapshot,
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

    fn failure_is_relevant(&self, command: &Command) -> bool {
        match &command.kind {
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
        && description.anchors == Anchors::TOP_RIGHT
        && description.logical_width == LOGICAL_WIDTH
        && description.logical_height == LOGICAL_HEIGHT
        && description.output_scale.is_finite()
        && description.output_scale > 0.0
        && description.output_scale <= MAX_SCALE
        && description.top_margin == TOP_MARGIN
        && description.right_margin == RIGHT_MARGIN
        && description.exclusive_zone == 0
        && description.keyboard_interactivity == KeyboardInteractivity::OnDemand
        && SeatId::new(description.invocation.seat.as_str()).is_ok()
        && !description.invocation.output.0.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(
        id: &str,
        enabled: bool,
        width: f64,
        height: f64,
        scale: f64,
    ) -> rmac_compositor::Output {
        rmac_compositor::Output {
            id: id.into(),
            make: "private make".into(),
            model: "private model".into(),
            serial: Some("private serial".into()),
            physical_size_mm: None,
            modes: Vec::new(),
            current_mode: enabled.then_some(0),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: enabled.then_some(rmac_compositor::LogicalOutput {
                position: rmac_compositor::LogicalPoint::default(),
                size: rmac_compositor::LogicalSize { width, height },
                scale,
                transform: "normal".into(),
            }),
        }
    }

    fn compositor(output_id: &str) -> rmac_compositor::Snapshot {
        let window = rmac_compositor::WindowId(7);
        rmac_compositor::Snapshot {
            outputs: vec![output(output_id, true, 1920.0, 1080.0, 1.5)],
            windows: vec![rmac_compositor::Window {
                id: window,
                title: None,
                app_id: None,
                pid: None,
                workspace: None,
                focused: true,
                floating: false,
                urgent: false,
                focus_timestamp: None,
                layout: rmac_compositor::WindowLayout::default(),
            }],
            focus: rmac_compositor::FocusState {
                output: Some(output_id.into()),
                window: Some(window),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn description(output: &str, seat: &str) -> Result<Description, PlanError> {
        plan(
            &output.into(),
            SeatId::new(seat).unwrap(),
            &compositor(output),
        )
    }

    fn apply_next(registry: &mut Registry) -> Command {
        let command = registry.next_command().unwrap().unwrap();
        registry.finish(command.id(), CommandResult::Applied);
        command
    }

    #[test]
    fn plan_targets_exact_output_and_captures_live_focus() {
        let snapshot = compositor("DP-2");
        let seats = rmac_shell_invocation::SeatInventory::new(vec!["seat-main".into()]).unwrap();
        let invocation = rmac_shell_invocation::global_shortcut(&snapshot, &seats).unwrap();
        let planned = plan_invocation(&invocation, &snapshot).unwrap();
        assert_eq!(planned.invocation.output.0, "DP-2");
        assert_eq!(planned.invocation.seat.as_str(), "seat-main");
        assert_eq!(
            planned.invocation.restore_window,
            Some(rmac_compositor::WindowId(7))
        );
        assert_eq!(planned.output_scale, 1.5);
        assert_eq!(planned.logical_width, LOGICAL_WIDTH);
        assert_eq!(planned.logical_height, LOGICAL_HEIGHT);
        assert_eq!(
            planned.keyboard_interactivity,
            KeyboardInteractivity::OnDemand
        );
    }

    #[test]
    fn focus_restore_revalidates_target_and_preserves_exact_seat() {
        let mut planned = description("DP-2", "private-seat-31").unwrap();
        let mut current = compositor("DP-2");
        assert_eq!(
            focus_restore(&planned, &current),
            FocusRestore::AlreadyFocused
        );

        current.focus.window = None;
        current.windows[0].focused = false;
        let request = focus_restore(&planned, &current);
        assert_eq!(
            request,
            FocusRestore::Request(FocusRestoreRequest {
                seat: SeatId::new("private-seat-31").unwrap(),
                window: rmac_compositor::WindowId(7),
            })
        );
        let diagnostics = format!("{request:?}");
        assert!(!diagnostics.contains("private-seat-31"));
        assert!(!diagnostics.contains('7'));

        current.windows.clear();
        assert_eq!(focus_restore(&planned, &current), FocusRestore::TargetGone);
        planned.invocation.restore_window = None;
        assert_eq!(
            focus_restore(&planned, &current),
            FocusRestore::NotRequested
        );
    }

    #[test]
    fn unavailable_invalid_and_small_outputs_fail_closed() {
        let seat = || SeatId::new("seat-main").unwrap();
        assert_eq!(
            plan(&"missing".into(), seat(), &compositor("DP-2")),
            Err(PlanError::OutputMissing)
        );
        let mut snapshot = compositor("DP-2");
        snapshot.outputs[0] = output("DP-2", false, 1920.0, 1080.0, 1.0);
        assert_eq!(
            plan(&"DP-2".into(), seat(), &snapshot),
            Err(PlanError::OutputDisabled)
        );
        snapshot.outputs[0] = output(
            "DP-2",
            true,
            LOGICAL_WIDTH + RIGHT_MARGIN - 1.0,
            LOGICAL_HEIGHT + TOP_MARGIN,
            1.0,
        );
        assert_eq!(
            plan(&"DP-2".into(), seat(), &snapshot),
            Err(PlanError::DoesNotFit)
        );
        snapshot.outputs[0] = output("DP-2", true, 1920.0, 1080.0, f64::NAN);
        assert_eq!(
            plan(&"DP-2".into(), seat(), &snapshot),
            Err(PlanError::InvalidGeometry)
        );

        let mut inconsistent = compositor("DP-2");
        inconsistent.windows[0].focused = false;
        assert_eq!(
            plan(&"DP-2".into(), seat(), &inconsistent)
                .unwrap()
                .invocation
                .restore_window,
            None
        );
    }

    #[test]
    fn create_transfer_close_and_acknowledgement_are_serial() {
        let mut registry = Registry::default();
        registry.open(&description("A", "seat-a")).unwrap();
        let create = apply_next(&mut registry);
        assert!(matches!(create.kind, CommandKind::Create { .. }));

        registry.open(&description("B", "seat-b")).unwrap();
        let remove = apply_next(&mut registry);
        assert!(matches!(remove.kind, CommandKind::Remove { .. }));
        let replacement = apply_next(&mut registry);
        assert!(matches!(replacement.kind, CommandKind::Create { .. }));
        assert_eq!(replacement.kind.output().0, "B");

        assert!(registry.close());
        assert!(matches!(
            apply_next(&mut registry).kind,
            CommandKind::Remove { .. }
        ));
        assert!(registry.next_command().unwrap().is_none());
    }

    #[test]
    fn newer_invocation_converges_after_inflight_work() {
        let mut registry = Registry::default();
        registry.open(&description("A", "seat-a")).unwrap();
        let stale = registry.next_command().unwrap().unwrap();
        registry.open(&description("B", "seat-b")).unwrap();
        assert!(registry.next_command().unwrap().is_none());
        registry.finish(stale.id(), CommandResult::Failed);
        let current = registry.next_command().unwrap().unwrap();
        assert_eq!(current.kind.output().0, "B");
    }

    #[test]
    fn relevant_failure_requires_retry_and_closed_surface_recreates() {
        let mut registry = Registry::default();
        registry.open(&description("A", "seat-a")).unwrap();
        let failed = registry.next_command().unwrap().unwrap();
        registry.finish(failed.id(), CommandResult::Failed);
        assert!(registry.next_command().unwrap().is_none());
        assert!(registry.retry().visible);

        let create = apply_next(&mut registry);
        let old_surface = create.kind.surface();
        assert!(registry.surface_closed(old_surface).visible);
        let replacement = registry.next_command().unwrap().unwrap();
        assert!(matches!(replacement.kind, CommandKind::Create { .. }));
        assert_ne!(replacement.kind.surface(), old_surface);
    }

    #[test]
    fn invalid_plan_preserves_state_and_diagnostics_are_private() {
        let mut registry = Registry::default();
        registry
            .open(&description("private-output-9", "private-seat-4"))
            .unwrap();
        let before = registry.snapshot();
        assert_eq!(
            registry.open(&Err(PlanError::OutputMissing)),
            Err(LifecycleError::InvalidPlan(PlanError::OutputMissing))
        );
        assert_eq!(registry.snapshot(), before);
        let diagnostics = format!("{:?}", registry.snapshot());
        assert!(!diagnostics.contains("private-output-9"));
        assert!(!diagnostics.contains("private-seat-4"));
    }
}
