//! Deterministic planning and acknowledged ownership for E2 banner surfaces.
//!
//! This module contains no GPUI or Wayland objects. The eventual Linux host
//! translates one [`SurfaceDescription`] into one compact layer surface and
//! acknowledges each [`Command`] only after the compositor operation is known.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use rmac_notifications::banner::{OutputId as BannerOutputId, Snapshot as LayoutSnapshot};
use rmac_notifications::NotificationId;
use rmac_notifications_runtime::presentation::{Card, ControlId, MAX_CARDS};

use crate::banner::Frame;

pub const NAMESPACE: &str = "rmac-notification-banners";
pub const MAX_NOTIFICATION_SURFACES: usize = 32;
pub const MAX_CARDS_PER_SURFACE: usize = 8;
pub const MIN_CARD_HEIGHT: u16 = 44;
pub const MAX_CARD_HEIGHT: u16 = 512;
pub const CARD_CORNER_RADIUS: u16 = 14;

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
    None,
    OnDemand,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CardMeasurement {
    pub notification: NotificationId,
    pub logical_height: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CardSlot {
    pub notification: NotificationId,
    pub stack_index: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, PartialEq)]
pub struct SurfaceDescription {
    pub output: rmac_compositor::OutputId,
    pub namespace: &'static str,
    pub layer: Layer,
    pub anchors: Anchors,
    pub logical_width: u32,
    pub logical_height: u32,
    pub output_scale: f64,
    pub top_margin: u16,
    pub right_margin: u16,
    pub exclusive_zone: i32,
    pub keyboard_interactivity: KeyboardInteractivity,
    pub gap: u16,
    pub cards: Vec<CardSlot>,
    /// A union of surface-local rectangles for `wl_surface.set_input_region`.
    /// Transparent gaps and rounded card corners remain click-through.
    pub input_regions: Vec<InputRegion>,
}

impl fmt::Debug for SurfaceDescription {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SurfaceDescription")
            .field("output", &"<redacted>")
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
            .field("gap", &self.gap)
            .field("cards", &self.cards)
            .field("input_regions", &self.input_regions)
            .finish()
    }
}

#[derive(Clone, Default, PartialEq)]
pub struct Plan {
    pub surfaces: Vec<SurfaceDescription>,
}

impl fmt::Debug for Plan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Plan")
            .field("surfaces", &self.surfaces)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlanError {
    TooManyCards { count: usize },
    TooManySurfaces { count: usize },
    TooManyCardsOnSurface { count: usize },
    DuplicateCard,
    DuplicateLayoutBanner,
    DuplicateMeasurement,
    DuplicateCompositorOutput,
    MissingCard,
    MissingLayoutBanner,
    MissingMeasurement,
    UnexpectedMeasurement,
    MismatchedCard,
    InvalidStackOrder,
    InvalidCardHeight,
    InvalidLayout,
    InvalidOutputGeometry,
    OutputUnavailable,
    SurfaceDoesNotFit,
    FocusedControlMissing,
    ArithmeticOverflow,
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "notification surface planning failed ({self:?})")
    }
}

impl std::error::Error for PlanError {}

/// Builds one compact top-right overlay surface for each output that currently
/// owns cards. The plan contains no notification text, application identity,
/// output metadata, icon source, or action target.
pub fn plan(
    frame: &Frame<'_>,
    compositor: &rmac_compositor::Snapshot,
    measurements: &[CardMeasurement],
    focused: Option<ControlId>,
) -> Result<Plan, PlanError> {
    validate_card_count(frame.cards.len())?;
    validate_card_count(frame.layout.banners.len())?;
    validate_card_count(measurements.len())?;
    validate_layout(&frame.layout)?;

    let cards = collect_cards(frame.cards)?;
    let banners = collect_banners(&frame.layout)?;
    validate_frame_join(&cards, &banners)?;
    let measurements = collect_measurements(measurements, &cards)?;
    let outputs = collect_outputs(compositor)?;

    let focused_output = focused
        .map(|focused| {
            let card = cards
                .get(&focused.notification())
                .copied()
                .ok_or(PlanError::FocusedControlMissing)?;
            if !card.controls.iter().any(|control| control.id == focused) {
                return Err(PlanError::FocusedControlMissing);
            }
            Ok(card.output.clone())
        })
        .transpose()?;

    let mut grouped = BTreeMap::<BannerOutputId, Vec<(NotificationId, usize, u16)>>::new();
    for card in frame.cards {
        let measurement = measurements
            .get(&card.id)
            .copied()
            .ok_or(PlanError::MissingMeasurement)?;
        grouped.entry(card.output.clone()).or_default().push((
            card.id,
            card.stack_index,
            measurement.logical_height,
        ));
    }
    if grouped.len() > MAX_NOTIFICATION_SURFACES {
        return Err(PlanError::TooManySurfaces {
            count: grouped.len(),
        });
    }

    let mut surfaces = Vec::with_capacity(grouped.len());
    for (banner_output, mut grouped_cards) in grouped {
        if grouped_cards.len() > MAX_CARDS_PER_SURFACE {
            return Err(PlanError::TooManyCardsOnSurface {
                count: grouped_cards.len(),
            });
        }
        grouped_cards.sort_by_key(|(_, stack_index, _)| *stack_index);
        if grouped_cards
            .iter()
            .enumerate()
            .any(|(expected, (_, actual, _))| expected != *actual)
        {
            return Err(PlanError::InvalidStackOrder);
        }

        let compositor_id = rmac_compositor::OutputId::from(banner_output.as_str());
        let output = outputs
            .get(&compositor_id)
            .copied()
            .ok_or(PlanError::OutputUnavailable)?;
        let logical = renderable_output(output).ok_or(PlanError::InvalidOutputGeometry)?;
        let description = description(
            &frame.layout,
            compositor_id,
            logical,
            &grouped_cards,
            focused_output.as_ref() == Some(&banner_output),
        )?;
        validate_description(&description)?;
        surfaces.push(description);
    }
    surfaces.sort_by(|left, right| left.output.cmp(&right.output));
    Ok(Plan { surfaces })
}

fn validate_card_count(count: usize) -> Result<(), PlanError> {
    if count > MAX_CARDS {
        return Err(PlanError::TooManyCards { count });
    }
    Ok(())
}

fn validate_layout(layout: &LayoutSnapshot) -> Result<(), PlanError> {
    if !(280..=480).contains(&layout.width_px) {
        return Err(PlanError::InvalidLayout);
    }
    Ok(())
}

fn collect_cards(cards: &[Card]) -> Result<BTreeMap<NotificationId, &Card>, PlanError> {
    let mut collected = BTreeMap::new();
    for card in cards {
        if collected.insert(card.id, card).is_some() {
            return Err(PlanError::DuplicateCard);
        }
    }
    Ok(collected)
}

fn collect_banners(
    layout: &LayoutSnapshot,
) -> Result<BTreeMap<NotificationId, &rmac_notifications::banner::BannerSnapshot>, PlanError> {
    let mut collected = BTreeMap::new();
    for banner in &layout.banners {
        if collected.insert(banner.id, banner).is_some() {
            return Err(PlanError::DuplicateLayoutBanner);
        }
    }
    Ok(collected)
}

fn validate_frame_join(
    cards: &BTreeMap<NotificationId, &Card>,
    banners: &BTreeMap<NotificationId, &rmac_notifications::banner::BannerSnapshot>,
) -> Result<(), PlanError> {
    if banners.keys().any(|id| !cards.contains_key(id)) {
        return Err(PlanError::MissingCard);
    }
    if cards.keys().any(|id| !banners.contains_key(id)) {
        return Err(PlanError::MissingLayoutBanner);
    }
    for (id, card) in cards {
        let banner = banners
            .get(id)
            .copied()
            .ok_or(PlanError::MissingLayoutBanner)?;
        if card.output != banner.output
            || card.phase != banner.phase
            || card.pause != banner.pause
            || card.stack_index != banner.stack_index
        {
            return Err(PlanError::MismatchedCard);
        }
    }
    Ok(())
}

fn collect_measurements(
    measurements: &[CardMeasurement],
    cards: &BTreeMap<NotificationId, &Card>,
) -> Result<BTreeMap<NotificationId, CardMeasurement>, PlanError> {
    let mut collected = BTreeMap::new();
    for measurement in measurements {
        if !(MIN_CARD_HEIGHT..=MAX_CARD_HEIGHT).contains(&measurement.logical_height) {
            return Err(PlanError::InvalidCardHeight);
        }
        if !cards.contains_key(&measurement.notification) {
            return Err(PlanError::UnexpectedMeasurement);
        }
        if collected
            .insert(measurement.notification, *measurement)
            .is_some()
        {
            return Err(PlanError::DuplicateMeasurement);
        }
    }
    if cards.keys().any(|id| !collected.contains_key(id)) {
        return Err(PlanError::MissingMeasurement);
    }
    Ok(collected)
}

fn collect_outputs(
    compositor: &rmac_compositor::Snapshot,
) -> Result<BTreeMap<rmac_compositor::OutputId, &rmac_compositor::Output>, PlanError> {
    let mut outputs = BTreeMap::new();
    for output in &compositor.outputs {
        if outputs.insert(output.id.clone(), output).is_some() {
            return Err(PlanError::DuplicateCompositorOutput);
        }
    }
    Ok(outputs)
}

fn renderable_output(output: &rmac_compositor::Output) -> Option<&rmac_compositor::LogicalOutput> {
    let logical = output.enabled().then_some(output.logical.as_ref()?)?;
    (logical.size.is_valid()
        && logical.size.width > 0.0
        && logical.size.height > 0.0
        && logical.scale.is_finite()
        && (crate::icon::MIN_OUTPUT_SCALE..=crate::icon::MAX_OUTPUT_SCALE).contains(&logical.scale))
    .then_some(logical)
}

fn description(
    layout: &LayoutSnapshot,
    output: rmac_compositor::OutputId,
    logical: &rmac_compositor::LogicalOutput,
    grouped_cards: &[(NotificationId, usize, u16)],
    focused: bool,
) -> Result<SurfaceDescription, PlanError> {
    let width = u32::from(layout.width_px);
    let gap = u32::from(layout.gap_px);
    let mut y = 0_u32;
    let mut cards = Vec::with_capacity(grouped_cards.len());
    let mut input_regions = Vec::with_capacity(grouped_cards.len().saturating_mul(2));
    for (position, (notification, stack_index, height)) in grouped_cards.iter().enumerate() {
        if position > 0 {
            y = y.checked_add(gap).ok_or(PlanError::ArithmeticOverflow)?;
        }
        let slot = CardSlot {
            notification: *notification,
            stack_index: *stack_index,
            x: 0,
            y,
            width,
            height: u32::from(*height),
        };
        input_regions.extend(card_input_regions(slot)?);
        y = y
            .checked_add(u32::from(*height))
            .ok_or(PlanError::ArithmeticOverflow)?;
        cards.push(slot);
    }
    if f64::from(width) + f64::from(layout.trailing_inset_px) > logical.size.width
        || f64::from(y) + f64::from(layout.top_inset_px) > logical.size.height
    {
        return Err(PlanError::SurfaceDoesNotFit);
    }
    Ok(SurfaceDescription {
        output,
        namespace: NAMESPACE,
        layer: Layer::Overlay,
        anchors: Anchors::TOP_RIGHT,
        logical_width: width,
        logical_height: y,
        output_scale: logical.scale,
        top_margin: layout.top_inset_px,
        right_margin: layout.trailing_inset_px,
        exclusive_zone: 0,
        keyboard_interactivity: if focused {
            KeyboardInteractivity::OnDemand
        } else {
            KeyboardInteractivity::None
        },
        gap: layout.gap_px,
        cards,
        input_regions,
    })
}

fn card_input_regions(card: CardSlot) -> Result<[InputRegion; 2], PlanError> {
    let radius = u32::from(CARD_CORNER_RADIUS);
    let double_radius = radius.checked_mul(2).ok_or(PlanError::ArithmeticOverflow)?;
    if card.width <= double_radius || card.height <= double_radius {
        return Err(PlanError::InvalidCardHeight);
    }
    Ok([
        InputRegion {
            x: card
                .x
                .checked_add(radius)
                .ok_or(PlanError::ArithmeticOverflow)?,
            y: card.y,
            width: card.width - double_radius,
            height: card.height,
        },
        InputRegion {
            x: card.x,
            y: card
                .y
                .checked_add(radius)
                .ok_or(PlanError::ArithmeticOverflow)?,
            width: card.width,
            height: card.height - double_radius,
        },
    ])
}

fn validate_description(description: &SurfaceDescription) -> Result<(), PlanError> {
    if description.namespace != NAMESPACE
        || description.layer != Layer::Overlay
        || description.anchors != Anchors::TOP_RIGHT
        || description.exclusive_zone != 0
        || !(280..=480).contains(&description.logical_width)
        || description.logical_height == 0
        || !description.output_scale.is_finite()
        || !(crate::icon::MIN_OUTPUT_SCALE..=crate::icon::MAX_OUTPUT_SCALE)
            .contains(&description.output_scale)
        || description.cards.is_empty()
        || description.cards.len() > MAX_CARDS_PER_SURFACE
    {
        return Err(PlanError::InvalidLayout);
    }
    let mut expected_y = 0_u32;
    let mut seen = BTreeSet::new();
    let mut expected_regions = Vec::with_capacity(description.cards.len().saturating_mul(2));
    for (position, card) in description.cards.iter().enumerate() {
        if position > 0 {
            expected_y = expected_y
                .checked_add(u32::from(description.gap))
                .ok_or(PlanError::ArithmeticOverflow)?;
        }
        if card.stack_index != position
            || card.x != 0
            || card.y != expected_y
            || card.width != description.logical_width
            || !(u32::from(MIN_CARD_HEIGHT)..=u32::from(MAX_CARD_HEIGHT)).contains(&card.height)
            || !seen.insert(card.notification)
        {
            return Err(PlanError::InvalidLayout);
        }
        expected_regions.extend(card_input_regions(*card)?);
        expected_y = expected_y
            .checked_add(card.height)
            .ok_or(PlanError::ArithmeticOverflow)?;
    }
    if expected_y != description.logical_height || expected_regions != description.input_regions {
        return Err(PlanError::InvalidLayout);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CommandId(u64);

impl CommandId {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SurfaceId(u64);

impl SurfaceId {
    pub fn get(self) -> u64 {
        self.0
    }
}

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
        description: SurfaceDescription,
    },
    Reconfigure {
        surface: SurfaceId,
        description: SurfaceDescription,
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

impl fmt::Debug for CommandKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct(match self {
                Self::Create { .. } => "Create",
                Self::Reconfigure { .. } => "Reconfigure",
                Self::Remove { .. } => "Remove",
            })
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
    pub output: rmac_compositor::OutputId,
    pub surface: SurfaceId,
    pub description: SurfaceDescription,
}

impl fmt::Debug for AppliedSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppliedSurface")
            .field("output", &"<redacted>")
            .field("surface", &self.surface)
            .field("description", &self.description)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
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

#[derive(Clone, Default, PartialEq)]
pub struct Snapshot {
    pub desired_outputs: Vec<rmac_compositor::OutputId>,
    pub applied: Vec<AppliedSurface>,
    pub pending: Option<Command>,
    pub failures: Vec<SurfaceFailure>,
}

impl fmt::Debug for Snapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Snapshot")
            .field("desired_output_count", &self.desired_outputs.len())
            .field("applied", &self.applied)
            .field("pending", &self.pending)
            .field("failures", &self.failures)
            .finish()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transition {
    pub snapshot: Snapshot,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LifecycleError {
    InvalidPlan(PlanError),
    TooManySurfaces { count: usize },
    DuplicateOutput,
    InvalidDescription,
    CommandExhausted,
    SurfaceExhausted,
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPlan(error) => write!(formatter, "the surface plan is invalid: {error}"),
            Self::TooManySurfaces { count } => write!(
                formatter,
                "the notification plan requested {count} surfaces; the limit is \
                 {MAX_NOTIFICATION_SURFACES}"
            ),
            Self::DuplicateOutput => {
                formatter.write_str("the notification surface plan repeats an output")
            }
            Self::InvalidDescription => {
                formatter.write_str("a notification surface description is invalid")
            }
            Self::CommandExhausted => {
                formatter.write_str("notification surface command identities are exhausted")
            }
            Self::SurfaceExhausted => {
                formatter.write_str("notification surface identities are exhausted")
            }
        }
    }
}

impl std::error::Error for LifecycleError {}

#[derive(Clone)]
struct Applied {
    surface: SurfaceId,
    description: SurfaceDescription,
}

#[derive(Default)]
pub struct Registry {
    desired: BTreeMap<rmac_compositor::OutputId, SurfaceDescription>,
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

    /// Accepts the newest complete plan. Any malformed plan leaves the
    /// previously accepted desired state untouched.
    pub fn set_desired(&mut self, plan: &Result<Plan, PlanError>) -> Result<bool, LifecycleError> {
        let plan = plan
            .as_ref()
            .map_err(|error| LifecycleError::InvalidPlan(*error))?;
        if plan.surfaces.len() > MAX_NOTIFICATION_SURFACES {
            return Err(LifecycleError::TooManySurfaces {
                count: plan.surfaces.len(),
            });
        }
        let mut desired = BTreeMap::new();
        for description in &plan.surfaces {
            validate_description(description).map_err(|_| LifecycleError::InvalidDescription)?;
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

    /// Produces at most one platform command. Applied state advances only after
    /// the host acknowledges that exact command identity.
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

    /// Cancels work only when the host knows the compositor was not touched.
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

    /// Reconciles an unsolicited layer-surface `closed` event. This covers
    /// output destruction and compositor-directed removal without pretending
    /// that an in-flight command completed.
    pub fn surface_closed(&mut self, surface: SurfaceId) -> Transition {
        let applied_output = self
            .applied
            .iter()
            .find_map(|(output, applied)| (applied.surface == surface).then(|| output.clone()));
        let pending_output = self
            .pending
            .as_ref()
            .filter(|command| command.kind.surface() == surface)
            .map(|command| command.kind.output().clone());
        if applied_output.is_none() && pending_output.is_none() {
            return Transition {
                snapshot: self.snapshot(),
                visible: false,
            };
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|command| command.kind.surface() == surface)
        {
            self.pending = None;
        }
        if let Some(output) = applied_output.as_ref() {
            self.applied.remove(output);
        }
        if let Some(output) = applied_output.or(pending_output) {
            self.failures.remove(&output);
        }
        Transition {
            snapshot: self.snapshot(),
            visible: true,
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
            CommandKind::Create { description, .. }
            | CommandKind::Reconfigure { description, .. } => self
                .desired
                .get(&description.output)
                .is_some_and(|desired| desired == description),
            CommandKind::Remove { output, .. } => !self.desired.contains_key(output),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_notifications::banner::{
        BannerSnapshot, PauseState, PhaseSnapshot, Snapshot as LayoutSnapshot,
    };
    use rmac_notifications::Priority;
    use rmac_notifications_runtime::presentation::{Control, ControlRole, LiveRegion};

    fn notification(value: u32) -> NotificationId {
        NotificationId::from_protocol(value).unwrap()
    }

    fn output_id(value: &str) -> BannerOutputId {
        BannerOutputId::parse(value).unwrap()
    }

    fn card(value: u32, output: &str, stack_index: usize) -> Card {
        let id = notification(value);
        Card {
            id,
            output: output_id(output),
            phase: PhaseSnapshot::Visible,
            pause: PauseState {
                hovered: false,
                keyboard_focused: false,
            },
            stack_index,
            app_id: "org.private.Application.8472".into(),
            app_name: "Private Application 8472".into(),
            title: "Private title 8472".into(),
            body: "Private body 8472".into(),
            priority: Priority::Normal,
            live_region: LiveRegion::Polite,
            accessible_label: "Private accessible content 8472".into(),
            controls: vec![
                Control {
                    id: ControlId::Card(id),
                    role: ControlRole::Notification,
                    label: "Private card 8472".into(),
                    accessible_label: "Private card label 8472".into(),
                    activatable: true,
                },
                Control {
                    id: ControlId::Dismiss(id),
                    role: ControlRole::Dismiss,
                    label: "Dismiss".into(),
                    accessible_label: "Dismiss private notification 8472".into(),
                    activatable: true,
                },
            ],
        }
    }

    fn frame(cards: &[Card]) -> Frame<'_> {
        Frame {
            layout: LayoutSnapshot {
                banners: cards
                    .iter()
                    .map(|card| BannerSnapshot {
                        id: card.id,
                        output: card.output.clone(),
                        phase: card.phase,
                        pause: card.pause,
                        stack_index: card.stack_index,
                    })
                    .collect(),
                top_inset_px: 12,
                trailing_inset_px: 12,
                gap_px: 10,
                width_px: 360,
            },
            cards,
            feedback: &[],
        }
    }

    fn compositor_output(id: &str, width: f64, height: f64, scale: f64) -> rmac_compositor::Output {
        rmac_compositor::Output {
            id: rmac_compositor::OutputId::from(id),
            make: "Private make 8472".into(),
            model: "Private model 8472".into(),
            serial: Some("Private serial 8472".into()),
            physical_size_mm: None,
            modes: vec![rmac_compositor::OutputMode {
                physical_size: rmac_compositor::PhysicalSize {
                    width: 1920,
                    height: 1080,
                },
                refresh_millihz: 60_000,
                preferred: true,
            }],
            current_mode: Some(0),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: Some(rmac_compositor::LogicalOutput {
                position: rmac_compositor::LogicalPoint::default(),
                size: rmac_compositor::LogicalSize { width, height },
                scale,
                transform: "normal".into(),
            }),
        }
    }

    fn compositor(outputs: Vec<rmac_compositor::Output>) -> rmac_compositor::Snapshot {
        rmac_compositor::Snapshot {
            outputs,
            ..rmac_compositor::Snapshot::default()
        }
    }

    fn measurements(cards: &[Card], heights: &[u16]) -> Vec<CardMeasurement> {
        cards
            .iter()
            .zip(heights)
            .map(|(card, height)| CardMeasurement {
                notification: card.id,
                logical_height: *height,
            })
            .collect()
    }

    fn single_plan(output: &str, focused: bool) -> Plan {
        let cards = vec![card(1, output, 0)];
        plan(
            &frame(&cards),
            &compositor(vec![compositor_output(output, 1920.0, 1080.0, 1.0)]),
            &measurements(&cards, &[100]),
            focused.then_some(ControlId::Card(cards[0].id)),
        )
        .unwrap()
    }

    fn apply_next(registry: &mut Registry) -> Command {
        let command = registry.next_command().unwrap().unwrap();
        assert!(
            registry
                .finish(command.id(), CommandResult::Applied)
                .visible
        );
        command
    }

    fn region_contains(region: InputRegion, x: u32, y: u32) -> bool {
        x >= region.x
            && x < region.x + region.width
            && y >= region.y
            && y < region.y + region.height
    }

    #[test]
    fn plan_builds_compact_top_right_overlay_with_click_through_gaps() {
        let cards = vec![
            card(1, "private-output-8472", 0),
            card(2, "private-output-8472", 1),
        ];
        let plan = plan(
            &frame(&cards),
            &compositor(vec![compositor_output(
                "private-output-8472",
                1920.0,
                1080.0,
                1.5,
            )]),
            &measurements(&cards, &[100, 120]),
            None,
        )
        .unwrap();

        assert_eq!(plan.surfaces.len(), 1);
        let surface = &plan.surfaces[0];
        assert_eq!(surface.namespace, NAMESPACE);
        assert_eq!(surface.layer, Layer::Overlay);
        assert_eq!(surface.anchors, Anchors::TOP_RIGHT);
        assert_eq!(surface.logical_width, 360);
        assert_eq!(surface.logical_height, 230);
        assert_eq!(surface.output_scale, 1.5);
        assert_eq!(surface.top_margin, 12);
        assert_eq!(surface.right_margin, 12);
        assert_eq!(surface.exclusive_zone, 0);
        assert_eq!(surface.keyboard_interactivity, KeyboardInteractivity::None);
        assert_eq!(surface.cards[0].y, 0);
        assert_eq!(surface.cards[1].y, 110);
        assert_eq!(surface.input_regions.len(), 4);
        assert!(surface
            .input_regions
            .iter()
            .any(|region| region_contains(*region, 180, 50)));
        assert!(!surface
            .input_regions
            .iter()
            .any(|region| region_contains(*region, 180, 105)));
        assert!(!surface
            .input_regions
            .iter()
            .any(|region| region_contains(*region, 0, 0)));
    }

    #[test]
    fn only_the_output_owning_focus_requests_on_demand_keyboard_input() {
        let cards = vec![card(1, "A", 0), card(2, "B", 0)];
        let plan = plan(
            &frame(&cards),
            &compositor(vec![
                compositor_output("A", 1920.0, 1080.0, 1.0),
                compositor_output("B", 2560.0, 1440.0, 2.0),
            ]),
            &measurements(&cards, &[100, 120]),
            Some(ControlId::Dismiss(cards[1].id)),
        )
        .unwrap();

        assert_eq!(plan.surfaces.len(), 2);
        assert_eq!(
            plan.surfaces[0].keyboard_interactivity,
            KeyboardInteractivity::None
        );
        assert_eq!(
            plan.surfaces[1].keyboard_interactivity,
            KeyboardInteractivity::OnDemand
        );
    }

    #[test]
    fn output_and_notification_content_are_absent_from_diagnostics() {
        let plan = single_plan("private-output-8472", false);
        let diagnostics = format!("{plan:?}");
        assert!(!diagnostics.contains("private-output-8472"));
        assert!(!diagnostics.contains("Private"));

        let mut registry = Registry::default();
        registry.set_desired(&Ok(plan)).unwrap();
        let command = apply_next(&mut registry);
        let diagnostics = format!("{:?} {:?}", command, registry.snapshot());
        assert!(!diagnostics.contains("private-output-8472"));
        assert!(!diagnostics.contains("Private"));
    }

    #[test]
    fn malformed_measurements_and_stale_focus_fail_atomically() {
        let cards = vec![card(1, "A", 0)];
        let compositor = compositor(vec![compositor_output("A", 1920.0, 1080.0, 1.0)]);
        let current_frame = frame(&cards);
        assert_eq!(
            plan(
                &current_frame,
                &compositor,
                &[CardMeasurement {
                    notification: cards[0].id,
                    logical_height: MIN_CARD_HEIGHT - 1,
                }],
                None,
            ),
            Err(PlanError::InvalidCardHeight)
        );
        assert_eq!(
            plan(&current_frame, &compositor, &[], None),
            Err(PlanError::MissingMeasurement)
        );
        assert_eq!(
            plan(
                &current_frame,
                &compositor,
                &[
                    CardMeasurement {
                        notification: cards[0].id,
                        logical_height: 100,
                    },
                    CardMeasurement {
                        notification: cards[0].id,
                        logical_height: 100,
                    },
                ],
                None,
            ),
            Err(PlanError::DuplicateMeasurement)
        );
        assert_eq!(
            plan(
                &current_frame,
                &compositor,
                &measurements(&cards, &[100]),
                Some(ControlId::Button {
                    notification: cards[0].id,
                    index: 99,
                }),
            ),
            Err(PlanError::FocusedControlMissing)
        );
    }

    #[test]
    fn mismatched_frame_and_non_contiguous_stacks_are_rejected() {
        let mut cards = vec![card(1, "A", 0)];
        let mut stale_layout = frame(&cards).layout;
        cards[0].stack_index = 1;
        let stale_frame = Frame {
            layout: stale_layout.clone(),
            cards: &cards,
            feedback: &[],
        };
        let compositor = compositor(vec![compositor_output("A", 1920.0, 1080.0, 1.0)]);
        assert_eq!(
            plan(
                &stale_frame,
                &compositor,
                &measurements(&cards, &[100]),
                None,
            ),
            Err(PlanError::MismatchedCard)
        );

        stale_layout.banners[0].stack_index = 1;
        let non_contiguous = Frame {
            layout: stale_layout,
            cards: &cards,
            feedback: &[],
        };
        assert_eq!(
            plan(
                &non_contiguous,
                &compositor,
                &measurements(&cards, &[100]),
                None,
            ),
            Err(PlanError::InvalidStackOrder)
        );
    }

    #[test]
    fn missing_invalid_and_too_small_outputs_fail_closed() {
        let cards = vec![card(1, "A", 0)];
        let measurements = measurements(&cards, &[100]);
        let current_frame = frame(&cards);
        assert_eq!(
            plan(&current_frame, &compositor(Vec::new()), &measurements, None,),
            Err(PlanError::OutputUnavailable)
        );
        assert_eq!(
            plan(
                &current_frame,
                &compositor(vec![compositor_output("A", 1920.0, 1080.0, 8.0)]),
                &measurements,
                None,
            ),
            Err(PlanError::InvalidOutputGeometry)
        );
        assert_eq!(
            plan(
                &current_frame,
                &compositor(vec![compositor_output("A", 370.0, 1080.0, 1.0)]),
                &measurements,
                None,
            ),
            Err(PlanError::SurfaceDoesNotFit)
        );
        assert_eq!(
            plan(
                &current_frame,
                &compositor(vec![compositor_output("A", 1920.0, 110.0, 1.0)]),
                &measurements,
                None,
            ),
            Err(PlanError::SurfaceDoesNotFit)
        );
    }

    #[test]
    fn registry_creates_in_stable_order_and_waits_for_acknowledgement() {
        let mut plan = single_plan("B", false);
        plan.surfaces.extend(single_plan("A", false).surfaces);
        let mut registry = Registry::default();
        assert!(registry.set_desired(&Ok(plan)).unwrap());

        let first = registry.next_command().unwrap().unwrap();
        assert_eq!(first.kind.output(), &rmac_compositor::OutputId::from("A"));
        assert!(matches!(&first.kind, CommandKind::Create { .. }));
        assert!(registry.next_command().unwrap().is_none());
        registry.finish(first.id(), CommandResult::Applied);
        let second = apply_next(&mut registry);
        assert_eq!(second.kind.output(), &rmac_compositor::OutputId::from("B"));
        assert_eq!(registry.snapshot().applied.len(), 2);
        assert!(registry.next_command().unwrap().is_none());
    }

    #[test]
    fn hotplug_removes_disconnected_surface_before_creating_replacement() {
        let mut registry = Registry::default();
        registry.set_desired(&Ok(single_plan("A", false))).unwrap();
        apply_next(&mut registry);
        registry.set_desired(&Ok(single_plan("B", false))).unwrap();

        let remove = apply_next(&mut registry);
        assert!(matches!(&remove.kind, CommandKind::Remove { .. }));
        assert_eq!(remove.kind.output(), &rmac_compositor::OutputId::from("A"));
        let create = apply_next(&mut registry);
        assert!(matches!(&create.kind, CommandKind::Create { .. }));
        assert_eq!(create.kind.output(), &rmac_compositor::OutputId::from("B"));
    }

    #[test]
    fn focus_and_geometry_changes_reconfigure_stable_surface_identity() {
        let mut registry = Registry::default();
        registry.set_desired(&Ok(single_plan("A", false))).unwrap();
        let create = apply_next(&mut registry);
        let surface = create.kind.surface();

        let focused = single_plan("A", true);
        registry.set_desired(&Ok(focused)).unwrap();
        let reconfigure = apply_next(&mut registry);
        assert!(matches!(&reconfigure.kind, CommandKind::Reconfigure { .. }));
        assert_eq!(reconfigure.kind.surface(), surface);
        assert_eq!(registry.snapshot().applied[0].surface, surface);
        assert_eq!(
            registry.snapshot().applied[0]
                .description
                .keyboard_interactivity,
            KeyboardInteractivity::OnDemand
        );
    }

    #[test]
    fn newer_desired_state_converges_after_pending_work_and_stale_ack_is_inert() {
        let mut registry = Registry::default();
        registry.set_desired(&Ok(single_plan("A", false))).unwrap();
        let create = registry.next_command().unwrap().unwrap();
        registry.set_desired(&Ok(single_plan("A", true))).unwrap();
        registry.finish(create.id(), CommandResult::Applied);

        let reconfigure = registry.next_command().unwrap().unwrap();
        assert!(matches!(&reconfigure.kind, CommandKind::Reconfigure { .. }));
        assert!(!registry.finish(create.id(), CommandResult::Failed).visible);
        assert_eq!(
            registry.snapshot().pending.as_ref().unwrap().id(),
            reconfigure.id()
        );
        registry.finish(reconfigure.id(), CommandResult::Applied);
        assert_eq!(
            registry.snapshot().applied[0]
                .description
                .keyboard_interactivity,
            KeyboardInteractivity::OnDemand
        );
    }

    #[test]
    fn one_failed_output_does_not_block_others_and_retry_is_exact() {
        let mut desired = single_plan("A", false);
        desired.surfaces.extend(single_plan("B", false).surfaces);
        let mut registry = Registry::default();
        registry.set_desired(&Ok(desired)).unwrap();
        let failed = registry.next_command().unwrap().unwrap();
        assert_eq!(failed.kind.output(), &rmac_compositor::OutputId::from("A"));
        registry.finish(failed.id(), CommandResult::Failed);
        assert_eq!(registry.snapshot().failures.len(), 1);

        let other = apply_next(&mut registry);
        assert_eq!(other.kind.output(), &rmac_compositor::OutputId::from("B"));
        assert!(registry.next_command().unwrap().is_none());
        assert!(
            registry
                .retry(&rmac_compositor::OutputId::from("A"))
                .visible
        );
        let retry = apply_next(&mut registry);
        assert_eq!(retry.kind.output(), &rmac_compositor::OutputId::from("A"));
        assert!(registry.snapshot().failures.is_empty());
    }

    #[test]
    fn failure_for_superseded_description_does_not_block_current_plan() {
        let mut registry = Registry::default();
        registry.set_desired(&Ok(single_plan("A", false))).unwrap();
        let old = registry.next_command().unwrap().unwrap();
        registry.set_desired(&Ok(single_plan("A", true))).unwrap();
        registry.finish(old.id(), CommandResult::Failed);
        assert!(registry.snapshot().failures.is_empty());

        let current = apply_next(&mut registry);
        let CommandKind::Create { description, .. } = current.kind else {
            panic!("current plan should create a fresh surface");
        };
        assert_eq!(
            description.keyboard_interactivity,
            KeyboardInteractivity::OnDemand
        );
    }

    #[test]
    fn compositor_closed_event_recreates_desired_surface_and_cancels_stale_work() {
        let mut registry = Registry::default();
        registry.set_desired(&Ok(single_plan("A", false))).unwrap();
        let first = apply_next(&mut registry);
        let first_surface = first.kind.surface();
        assert!(registry.surface_closed(first_surface).visible);
        assert!(registry.snapshot().applied.is_empty());
        assert!(!registry.surface_closed(first_surface).visible);

        let replacement = apply_next(&mut registry);
        assert_ne!(replacement.kind.surface(), first_surface);
        registry.set_desired(&Ok(single_plan("A", true))).unwrap();
        let pending = registry.next_command().unwrap().unwrap();
        let current_surface = pending.kind.surface();
        assert!(registry.surface_closed(current_surface).visible);
        assert!(registry.snapshot().pending.is_none());
        assert!(registry.snapshot().applied.is_empty());
        assert!(
            !registry
                .finish(pending.id(), CommandResult::Applied)
                .visible
        );
        assert_ne!(apply_next(&mut registry).kind.surface(), current_surface);
    }

    #[test]
    fn invalid_plan_preserves_last_accepted_desired_state() {
        let mut registry = Registry::default();
        registry.set_desired(&Ok(single_plan("A", false))).unwrap();
        assert_eq!(
            registry.set_desired(&Err(PlanError::InvalidLayout)),
            Err(LifecycleError::InvalidPlan(PlanError::InvalidLayout))
        );
        assert_eq!(
            registry.snapshot().desired_outputs,
            vec![rmac_compositor::OutputId::from("A")]
        );

        let mut forged = single_plan("B", false);
        forged.surfaces[0].input_regions.clear();
        assert_eq!(
            registry.set_desired(&Ok(forged)),
            Err(LifecycleError::InvalidDescription)
        );
        assert_eq!(
            registry.snapshot().desired_outputs,
            vec![rmac_compositor::OutputId::from("A")]
        );
    }

    #[test]
    fn identity_exhaustion_is_explicit_and_does_not_wrap() {
        let mut registry = Registry {
            next_command: u64::MAX,
            ..Registry::default()
        };
        registry.set_desired(&Ok(single_plan("A", false))).unwrap();
        assert_eq!(
            registry.next_command(),
            Err(LifecycleError::CommandExhausted)
        );
        assert!(registry.snapshot().pending.is_none());

        let mut registry = Registry {
            next_surface: u64::MAX,
            ..Registry::default()
        };
        registry.set_desired(&Ok(single_plan("A", false))).unwrap();
        assert_eq!(
            registry.next_command(),
            Err(LifecycleError::SurfaceExhausted)
        );
        assert!(registry.snapshot().pending.is_none());
    }
}
