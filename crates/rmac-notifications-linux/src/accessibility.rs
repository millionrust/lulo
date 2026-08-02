//! Bounded, privacy-safe semantics for the per-output notification banners.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use rmac_notifications::banner::{OutputId, PhaseSnapshot};
use rmac_notifications::{NotificationId, Priority};
use rmac_notifications_runtime::presentation::{
    Announcement, Control, ControlId, ControlRole, LiveRegion, MAX_CARDS,
};

use crate::banner::{Feedback, FeedbackId, Frame, MAX_RETAINED_FEEDBACK};

pub const BANNER_REGION_NAME: &str = "Notifications";
pub const DISMISS_ERROR_LABEL: &str = "Dismiss error";
pub const MAX_ACCESSIBLE_SURFACES: usize = 32;
pub const MAX_ACCESSIBLE_CARDS_PER_SURFACE: usize = 8;
pub const MAX_ACCESSIBLE_CONTROLS_PER_CARD: usize = 10;
pub const MAX_ACCESSIBLE_TEXT_VALUE_BYTES: usize = 20 * 1024;
pub const MAX_ACCESSIBLE_TEXT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibleRole {
    Region,
    Notification,
    Button,
    Status,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibleActionKind {
    InvokeDefault,
    InvokeButton(usize),
    DismissNotification,
    DismissFeedback,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum AccessibleTarget {
    Control(ControlId),
    Feedback(FeedbackId),
}

impl fmt::Debug for AccessibleTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Control(_) => "AccessibleTarget::Control(<redacted>)",
            Self::Feedback(_) => "AccessibleTarget::Feedback(<redacted>)",
        })
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleAction {
    pub id: String,
    pub name: String,
    pub kind: AccessibleActionKind,
    pub target: AccessibleTarget,
    pub enabled: bool,
    pub busy: bool,
}

impl fmt::Debug for AccessibleAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleAction")
            .field("id", &self.id)
            .field("name", &"<redacted>")
            .field("kind", &self.kind)
            .field("target", &self.target)
            .field("enabled", &self.enabled)
            .field("busy", &self.busy)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleButton {
    pub id: String,
    pub role: AccessibleRole,
    pub name: String,
    pub focused: bool,
    pub action: AccessibleAction,
}

impl fmt::Debug for AccessibleButton {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleButton")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("name", &"<redacted>")
            .field("focused", &self.focused)
            .field("action", &self.action)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleCard {
    pub id: String,
    pub role: AccessibleRole,
    pub name: String,
    pub application_name: String,
    pub title: String,
    pub body: String,
    pub priority: Priority,
    pub live_region: LiveRegion,
    pub phase: PhaseSnapshot,
    pub paused: bool,
    pub position_in_set: usize,
    pub set_size: usize,
    pub focused: bool,
    pub busy: bool,
    pub default_action: Option<AccessibleAction>,
    pub buttons: Vec<AccessibleButton>,
}

impl fmt::Debug for AccessibleCard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleCard")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("name", &"<redacted>")
            .field("application_name", &"<redacted>")
            .field("title", &"<redacted>")
            .field("body", &"<redacted>")
            .field("priority", &self.priority)
            .field("live_region", &self.live_region)
            .field("phase", &self.phase)
            .field("paused", &self.paused)
            .field("position_in_set", &self.position_in_set)
            .field("set_size", &self.set_size)
            .field("focused", &self.focused)
            .field("busy", &self.busy)
            .field("has_default_action", &self.default_action.is_some())
            .field("button_count", &self.buttons.len())
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleSurface {
    /// Exact private host target. Diagnostics deliberately redact it.
    pub output: OutputId,
    pub id: String,
    pub role: AccessibleRole,
    pub name: &'static str,
    pub position_in_set: usize,
    pub set_size: usize,
    pub cards: Vec<AccessibleCard>,
}

impl fmt::Debug for AccessibleSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleSurface")
            .field("output", &"<redacted>")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("name", &self.name)
            .field("position_in_set", &self.position_in_set)
            .field("set_size", &self.set_size)
            .field("card_count", &self.cards.len())
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleFeedback {
    pub id: String,
    pub role: AccessibleRole,
    pub title: &'static str,
    pub message: String,
    pub live_region: LiveRegion,
    pub position_in_set: usize,
    pub set_size: usize,
    pub dismiss_action: AccessibleAction,
}

impl fmt::Debug for AccessibleFeedback {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleFeedback")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("title", &self.title)
            .field("message", &"<redacted>")
            .field("live_region", &self.live_region)
            .field("position_in_set", &self.position_in_set)
            .field("set_size", &self.set_size)
            .field("dismiss_action", &self.dismiss_action)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct LiveAnnouncement {
    pub id: String,
    pub text: String,
    pub politeness: LiveRegion,
}

impl fmt::Debug for LiveAnnouncement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveAnnouncement")
            .field("id", &self.id)
            .field("text", &"<redacted>")
            .field("politeness", &self.politeness)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct BannerAccessibilitySnapshot {
    pub name: &'static str,
    pub surfaces: Vec<AccessibleSurface>,
    pub feedback: Vec<AccessibleFeedback>,
    /// Exact focus targets in the visual tree's reading order. The host must
    /// not move focus here merely because a banner appeared.
    pub keyboard_order: Vec<AccessibleTarget>,
    pub focused: Option<AccessibleTarget>,
    pub announcements: Vec<LiveAnnouncement>,
}

impl fmt::Debug for BannerAccessibilitySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BannerAccessibilitySnapshot")
            .field("surface_count", &self.surfaces.len())
            .field(
                "card_count",
                &self
                    .surfaces
                    .iter()
                    .map(|surface| surface.cards.len())
                    .sum::<usize>(),
            )
            .field("feedback_count", &self.feedback.len())
            .field("keyboard_target_count", &self.keyboard_order.len())
            .field("has_focus", &self.focused.is_some())
            .field("announcement_count", &self.announcements.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibilityProjectionError {
    CardLimit,
    SurfaceLimit,
    SurfaceCardLimit,
    ControlLimit,
    FeedbackLimit,
    InvalidLayout,
    DuplicateCard,
    DuplicateControl,
    DuplicateFeedback,
    DuplicateAnnouncement,
    MismatchedCard,
    InvalidStackOrder,
    InvalidControl,
    InvalidFocus,
    InvalidPendingControl,
    InvalidAnnouncement,
    InvalidText,
    TextValueLimit,
    TextLimit,
}

/// Projects the exact accepted renderer frame. The projection never infers an
/// action from visible text and never puts an output, notification, or action
/// target into custom diagnostics.
pub fn project_banner_accessibility(
    frame: &Frame<'_>,
    focused: Option<ControlId>,
    pending: Option<ControlId>,
    announcements: &[Announcement],
) -> Result<BannerAccessibilitySnapshot, AccessibilityProjectionError> {
    validate_bounds(frame)?;
    let mut budget = TextBudget::default();
    budget.add_required(BANNER_REGION_NAME, false)?;

    let mut card_ids = BTreeSet::new();
    let mut controls = BTreeSet::new();
    let mut grouped = BTreeMap::<OutputId, Vec<AccessibleCard>>::new();
    for (card, banner) in frame.cards.iter().zip(&frame.layout.banners) {
        if !card_ids.insert(card.id) {
            return Err(AccessibilityProjectionError::DuplicateCard);
        }
        validate_card_join(card, banner)?;
        let accessible = project_card(card, focused, pending, &mut controls, &mut budget)?;
        grouped
            .entry(card.output.clone())
            .or_default()
            .push(accessible);
    }
    if grouped.len() > MAX_ACCESSIBLE_SURFACES {
        return Err(AccessibilityProjectionError::SurfaceLimit);
    }

    if let Some(target) = focused {
        if !controls.contains(&target)
            || frame
                .cards
                .iter()
                .filter(|card| card.pause.keyboard_focused)
                .map(|card| card.id)
                .ne(std::iter::once(target.notification()))
        {
            return Err(AccessibilityProjectionError::InvalidFocus);
        }
    } else if frame.cards.iter().any(|card| card.pause.keyboard_focused) {
        return Err(AccessibilityProjectionError::InvalidFocus);
    }
    if pending.is_some_and(|target| {
        !controls.contains(&target) || !control_is_activatable(frame.cards, target)
    }) {
        return Err(AccessibilityProjectionError::InvalidPendingControl);
    }

    for cards in grouped.values_mut() {
        let set_size = cards.len();
        for card in cards {
            card.set_size = set_size;
        }
    }

    let surface_count = grouped.len();
    let mut surfaces = Vec::with_capacity(surface_count);
    for (surface_index, (output, cards)) in grouped.into_iter().enumerate() {
        if cards.len() > MAX_ACCESSIBLE_CARDS_PER_SURFACE {
            return Err(AccessibilityProjectionError::SurfaceCardLimit);
        }
        if cards
            .iter()
            .enumerate()
            .any(|(index, card)| card.position_in_set != index + 1 || card.set_size != cards.len())
        {
            return Err(AccessibilityProjectionError::InvalidStackOrder);
        }
        surfaces.push(AccessibleSurface {
            output,
            id: format!("notification-surface-{surface_index}"),
            role: AccessibleRole::Region,
            name: BANNER_REGION_NAME,
            position_in_set: surface_index + 1,
            set_size: surface_count,
            cards,
        });
    }

    let mut keyboard_order = frame
        .cards
        .iter()
        .flat_map(|card| card.controls.iter())
        .map(|control| AccessibleTarget::Control(control.id))
        .collect::<Vec<_>>();
    let feedback = project_feedback(frame.feedback, &mut keyboard_order, &mut budget)?;
    if keyboard_order.iter().collect::<BTreeSet<_>>().len() != keyboard_order.len() {
        return Err(AccessibilityProjectionError::DuplicateControl);
    }

    let announcements = project_announcements(announcements, frame.cards, &card_ids, &mut budget)?;

    Ok(BannerAccessibilitySnapshot {
        name: BANNER_REGION_NAME,
        surfaces,
        feedback,
        keyboard_order,
        focused: focused.map(AccessibleTarget::Control),
        announcements,
    })
}

fn validate_bounds(frame: &Frame<'_>) -> Result<(), AccessibilityProjectionError> {
    if frame.cards.len() > MAX_CARDS || frame.layout.banners.len() > MAX_CARDS {
        return Err(AccessibilityProjectionError::CardLimit);
    }
    if frame.cards.len() != frame.layout.banners.len()
        || !(280..=480).contains(&frame.layout.width_px)
    {
        return Err(AccessibilityProjectionError::InvalidLayout);
    }
    if frame.feedback.len() > MAX_RETAINED_FEEDBACK {
        return Err(AccessibilityProjectionError::FeedbackLimit);
    }
    let mut previous_output = None;
    let mut expected_stack_index = 0;
    for banner in &frame.layout.banners {
        match previous_output {
            Some(output) if output == &banner.output => expected_stack_index += 1,
            Some(output) if output > &banner.output => {
                return Err(AccessibilityProjectionError::InvalidStackOrder);
            }
            _ => expected_stack_index = 0,
        }
        if banner.stack_index != expected_stack_index {
            return Err(AccessibilityProjectionError::InvalidStackOrder);
        }
        previous_output = Some(&banner.output);
    }
    Ok(())
}

fn validate_card_join(
    card: &rmac_notifications_runtime::presentation::Card,
    banner: &rmac_notifications::banner::BannerSnapshot,
) -> Result<(), AccessibilityProjectionError> {
    if card.id != banner.id
        || card.output != banner.output
        || card.phase != banner.phase
        || card.pause != banner.pause
        || card.stack_index != banner.stack_index
    {
        return Err(AccessibilityProjectionError::MismatchedCard);
    }
    Ok(())
}

fn project_card(
    card: &rmac_notifications_runtime::presentation::Card,
    focused: Option<ControlId>,
    pending: Option<ControlId>,
    seen_controls: &mut BTreeSet<ControlId>,
    budget: &mut TextBudget,
) -> Result<AccessibleCard, AccessibilityProjectionError> {
    if card.controls.is_empty() || card.controls.len() > MAX_ACCESSIBLE_CONTROLS_PER_CARD {
        return Err(AccessibilityProjectionError::ControlLimit);
    }
    budget.add_required(&card.app_name, false)?;
    budget.add_optional(&card.title, false)?;
    budget.add_optional(&card.body, true)?;
    budget.add_required(&card.accessible_label, true)?;

    let primary = &card.controls[0];
    if primary.id != ControlId::Card(card.id) || primary.role != ControlRole::Notification {
        return Err(AccessibilityProjectionError::InvalidControl);
    }
    validate_control_text(primary, budget)?;
    if !seen_controls.insert(primary.id) {
        return Err(AccessibilityProjectionError::DuplicateControl);
    }
    let primary_target = AccessibleTarget::Control(primary.id);
    let default_action = primary
        .activatable
        .then(|| {
            accessible_action(
                format!("notification-{}-default", card.id.get()),
                &primary.accessible_label,
                AccessibleActionKind::InvokeDefault,
                primary_target,
                pending.is_none(),
                pending == Some(primary.id),
                budget,
            )
        })
        .transpose()?;

    let mut buttons = Vec::with_capacity(card.controls.len().saturating_sub(1));
    let mut saw_dismiss = false;
    let mut previous_button = None;
    for control in card.controls.iter().skip(1) {
        validate_control_text(control, budget)?;
        if !control.activatable || !seen_controls.insert(control.id) {
            return Err(AccessibilityProjectionError::InvalidControl);
        }
        let kind = match (control.role, control.id) {
            (
                ControlRole::Action,
                ControlId::Button {
                    notification,
                    index,
                },
            ) if notification == card.id && !saw_dismiss => {
                if previous_button.is_some_and(|previous| index <= previous) {
                    return Err(AccessibilityProjectionError::InvalidControl);
                }
                previous_button = Some(index);
                AccessibleActionKind::InvokeButton(index)
            }
            (ControlRole::Dismiss, ControlId::Dismiss(notification))
                if notification == card.id && !saw_dismiss =>
            {
                saw_dismiss = true;
                AccessibleActionKind::DismissNotification
            }
            _ => return Err(AccessibilityProjectionError::InvalidControl),
        };
        let target = AccessibleTarget::Control(control.id);
        let id = control_semantic_id(control.id);
        let action = accessible_action(
            id.clone(),
            &control.accessible_label,
            kind,
            target,
            pending.is_none(),
            pending == Some(control.id),
            budget,
        )?;
        buttons.push(AccessibleButton {
            id,
            role: AccessibleRole::Button,
            name: control.accessible_label.clone(),
            focused: focused == Some(control.id),
            action,
        });
    }

    let set_size = card.stack_index.saturating_add(1).max(1);
    Ok(AccessibleCard {
        id: format!("notification-{}", card.id.get()),
        role: AccessibleRole::Notification,
        name: card.accessible_label.clone(),
        application_name: card.app_name.clone(),
        title: card.title.clone(),
        body: card.body.clone(),
        priority: card.priority,
        live_region: card.live_region,
        phase: card.phase,
        paused: card.pause.hovered || card.pause.keyboard_focused,
        position_in_set: card.stack_index + 1,
        // Corrected after grouping, before the value is validated/exported.
        set_size,
        focused: focused == Some(primary.id),
        busy: pending == Some(primary.id),
        default_action,
        buttons,
    })
}

fn project_feedback(
    feedback: &[Feedback],
    keyboard_order: &mut Vec<AccessibleTarget>,
    budget: &mut TextBudget,
) -> Result<Vec<AccessibleFeedback>, AccessibilityProjectionError> {
    let mut seen = BTreeSet::new();
    let mut seen_scopes = Vec::new();
    let mut previous = None;
    let set_size = feedback.len();
    let mut projected = Vec::with_capacity(set_size);
    for (index, item) in feedback.iter().enumerate() {
        let scope = (item.notification, item.operation);
        if !seen.insert(item.id)
            || seen_scopes.contains(&scope)
            || previous.is_some_and(|previous| item.id <= previous)
        {
            return Err(AccessibilityProjectionError::DuplicateFeedback);
        }
        seen_scopes.push(scope);
        previous = Some(item.id);
        let title = item.operation.title();
        let message = item.reason.message().to_owned();
        budget.add_required(title, false)?;
        budget.add_required(&message, false)?;
        budget.add_required(DISMISS_ERROR_LABEL, false)?;
        let target = AccessibleTarget::Feedback(item.id);
        let action = accessible_action(
            format!("notification-feedback-{}-dismiss", item.id.get()),
            DISMISS_ERROR_LABEL,
            AccessibleActionKind::DismissFeedback,
            target,
            true,
            false,
            budget,
        )?;
        keyboard_order.push(target);
        projected.push(AccessibleFeedback {
            id: format!("notification-feedback-{}", item.id.get()),
            role: AccessibleRole::Status,
            title,
            message,
            live_region: item.live_region(),
            position_in_set: index + 1,
            set_size,
            dismiss_action: action,
        });
    }
    Ok(projected)
}

fn project_announcements(
    announcements: &[Announcement],
    cards: &[rmac_notifications_runtime::presentation::Card],
    card_ids: &BTreeSet<NotificationId>,
    budget: &mut TextBudget,
) -> Result<Vec<LiveAnnouncement>, AccessibilityProjectionError> {
    let mut seen = BTreeSet::new();
    let mut projected = Vec::with_capacity(announcements.len());
    for announcement in announcements {
        if !seen.insert(announcement.id) {
            return Err(AccessibilityProjectionError::DuplicateAnnouncement);
        }
        if !card_ids.contains(&announcement.id) {
            return Err(AccessibilityProjectionError::InvalidAnnouncement);
        }
        let card = cards
            .iter()
            .find(|card| card.id == announcement.id)
            .ok_or(AccessibilityProjectionError::InvalidAnnouncement)?;
        if card.live_region != announcement.live_region
            || matches!(card.phase, PhaseSnapshot::Exiting(_))
        {
            return Err(AccessibilityProjectionError::InvalidAnnouncement);
        }
        budget.add_required(&card.accessible_label, true)?;
        projected.push(LiveAnnouncement {
            id: format!("notification-{}-announcement", card.id.get()),
            text: card.accessible_label.clone(),
            politeness: announcement.live_region,
        });
    }
    Ok(projected)
}

fn validate_control_text(
    control: &Control,
    budget: &mut TextBudget,
) -> Result<(), AccessibilityProjectionError> {
    budget.add_required(&control.label, false)?;
    budget.add_required(&control.accessible_label, true)
}

fn control_is_activatable(
    cards: &[rmac_notifications_runtime::presentation::Card],
    target: ControlId,
) -> bool {
    cards
        .iter()
        .flat_map(|card| &card.controls)
        .any(|control| control.id == target && control.activatable)
}

fn accessible_action(
    id: String,
    name: &str,
    kind: AccessibleActionKind,
    target: AccessibleTarget,
    enabled: bool,
    busy: bool,
    budget: &mut TextBudget,
) -> Result<AccessibleAction, AccessibilityProjectionError> {
    if enabled && busy {
        return Err(AccessibilityProjectionError::InvalidPendingControl);
    }
    budget.add_required(&id, false)?;
    budget.add_required(name, true)?;
    Ok(AccessibleAction {
        id,
        name: name.to_owned(),
        kind,
        target,
        enabled,
        busy,
    })
}

fn control_semantic_id(control: ControlId) -> String {
    match control {
        ControlId::Card(notification) => format!("notification-{}", notification.get()),
        ControlId::Button {
            notification,
            index,
        } => format!("notification-{}-button-{index}", notification.get()),
        ControlId::Dismiss(notification) => {
            format!("notification-{}-dismiss", notification.get())
        }
    }
}

#[derive(Default)]
struct TextBudget {
    bytes: usize,
}

impl TextBudget {
    fn add_required(
        &mut self,
        text: &str,
        multiline: bool,
    ) -> Result<(), AccessibilityProjectionError> {
        if text.trim().is_empty() {
            return Err(AccessibilityProjectionError::InvalidText);
        }
        self.add_optional(text, multiline)
    }

    fn add_optional(
        &mut self,
        text: &str,
        multiline: bool,
    ) -> Result<(), AccessibilityProjectionError> {
        if text.len() > MAX_ACCESSIBLE_TEXT_VALUE_BYTES {
            return Err(AccessibilityProjectionError::TextValueLimit);
        }
        if text.chars().any(|character| {
            character.is_control() && !(multiline && matches!(character, '\n' | '\t'))
        }) {
            return Err(AccessibilityProjectionError::InvalidText);
        }
        self.bytes = self.bytes.saturating_add(text.len());
        if self.bytes > MAX_ACCESSIBLE_TEXT_BYTES {
            return Err(AccessibilityProjectionError::TextLimit);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_notifications::banner::{
        BannerSnapshot, CloseCause, PauseState, Snapshot as LayoutSnapshot,
    };
    use rmac_notifications_runtime::presentation::{Card, Control};

    fn id(value: u32) -> NotificationId {
        NotificationId::from_protocol(value).unwrap()
    }

    fn output(value: &str) -> OutputId {
        OutputId::parse(value).unwrap()
    }

    fn card(notification: NotificationId, output: OutputId, index: usize) -> Card {
        Card {
            id: notification,
            output,
            phase: PhaseSnapshot::Visible,
            pause: PauseState {
                hovered: false,
                keyboard_focused: false,
            },
            stack_index: index,
            app_id: "org.example.Private".into(),
            app_name: "Private Chat 8472".into(),
            title: "Private title 8472".into(),
            body: "Private body 8472".into(),
            priority: Priority::Urgent,
            live_region: LiveRegion::Assertive,
            accessible_label:
                "Urgent notification from Private Chat 8472. Private title 8472. Private body 8472"
                    .into(),
            controls: vec![
                Control {
                    id: ControlId::Card(notification),
                    role: ControlRole::Notification,
                    label: "Private title 8472".into(),
                    accessible_label: "Open private notification 8472".into(),
                    activatable: true,
                },
                Control {
                    id: ControlId::Button {
                        notification,
                        index: 0,
                    },
                    role: ControlRole::Action,
                    label: "Reply 8472".into(),
                    accessible_label: "Reply 8472".into(),
                    activatable: true,
                },
                Control {
                    id: ControlId::Dismiss(notification),
                    role: ControlRole::Dismiss,
                    label: "Dismiss".into(),
                    accessible_label: "Dismiss private notification 8472".into(),
                    activatable: true,
                },
            ],
        }
    }

    fn frame<'a>(cards: &'a [Card], banners: Vec<BannerSnapshot>) -> Frame<'a> {
        Frame {
            layout: LayoutSnapshot {
                banners,
                top_inset_px: 12,
                trailing_inset_px: 12,
                gap_px: 10,
                width_px: 360,
            },
            cards,
            feedback: &[],
        }
    }

    fn banner(card: &Card) -> BannerSnapshot {
        BannerSnapshot {
            id: card.id,
            output: card.output.clone(),
            phase: card.phase,
            pause: card.pause,
            stack_index: card.stack_index,
        }
    }

    #[test]
    fn projects_multi_output_cards_actions_focus_and_announcement_without_debug_leaks() {
        let first_output = output("private-output-a-8472");
        let second_output = output("private-output-b-8472");
        let mut cards = vec![card(id(1), first_output, 0), card(id(2), second_output, 0)];
        cards[0].pause.keyboard_focused = true;
        let banners = cards.iter().map(banner).collect();
        let frame = frame(&cards, banners);
        let focused = ControlId::Button {
            notification: id(1),
            index: 0,
        };
        let projected = project_banner_accessibility(
            &frame,
            Some(focused),
            Some(focused),
            &[Announcement {
                id: id(2),
                live_region: LiveRegion::Assertive,
            }],
        )
        .unwrap();

        assert_eq!(projected.surfaces.len(), 2);
        assert_eq!(projected.surfaces[0].role, AccessibleRole::Region);
        assert_eq!(projected.surfaces[0].cards[0].position_in_set, 1);
        assert!(projected.surfaces[0].cards[0].paused);
        assert_eq!(projected.focused, Some(AccessibleTarget::Control(focused)));
        assert!(projected.surfaces[0].cards[0].default_action.is_some());
        assert!(projected.surfaces[0].cards[0]
            .buttons
            .iter()
            .all(|button| !button.action.enabled));
        assert!(projected.surfaces[0].cards[0].buttons[0].action.busy);
        assert_eq!(projected.announcements.len(), 1);
        assert_eq!(projected.announcements[0].politeness, LiveRegion::Assertive);

        let diagnostics = format!("{projected:?}");
        for private in [
            "private-output",
            "Private Chat",
            "Private title",
            "Private body",
            "Reply 8472",
        ] {
            assert!(!diagnostics.contains(private));
        }
    }

    #[test]
    fn rejects_mismatched_frames_controls_focus_and_terminal_announcements() {
        let mut cards = vec![card(id(1), output("private-output"), 0)];
        let mut banners = cards.iter().map(banner).collect::<Vec<_>>();
        banners[0].phase = PhaseSnapshot::Entering;
        assert_eq!(
            project_banner_accessibility(&frame(&cards, banners), None, None, &[]),
            Err(AccessibilityProjectionError::MismatchedCard)
        );

        cards[0].controls[1].id = ControlId::Dismiss(id(1));
        let banners = cards.iter().map(banner).collect();
        assert_eq!(
            project_banner_accessibility(&frame(&cards, banners), None, None, &[]),
            Err(AccessibilityProjectionError::InvalidControl)
        );

        let mut cards = vec![card(id(1), output("private-output"), 0)];
        let banners = cards.iter().map(banner).collect();
        assert_eq!(
            project_banner_accessibility(
                &frame(&cards, banners),
                Some(ControlId::Card(id(1))),
                None,
                &[],
            ),
            Err(AccessibilityProjectionError::InvalidFocus)
        );

        cards[0].phase = PhaseSnapshot::Exiting(CloseCause::Authority);
        let banners = cards.iter().map(banner).collect();
        assert_eq!(
            project_banner_accessibility(
                &frame(&cards, banners),
                None,
                None,
                &[Announcement {
                    id: id(1),
                    live_region: LiveRegion::Assertive,
                }],
            ),
            Err(AccessibilityProjectionError::InvalidAnnouncement)
        );

        let cards = vec![
            card(id(1), output("z-private-output"), 0),
            card(id(2), output("a-private-output"), 0),
        ];
        let banners = cards.iter().map(banner).collect();
        assert_eq!(
            project_banner_accessibility(&frame(&cards, banners), None, None, &[]),
            Err(AccessibilityProjectionError::InvalidStackOrder)
        );
    }

    #[test]
    fn text_and_card_limits_fail_closed() {
        let mut session = crate::banner::BannerSession::new(
            rmac_notifications::banner::Config::default(),
            rmac_notifications::banner::PlacementPolicy::ActiveOutput,
            &rmac_appearance::Snapshot::default(),
        )
        .unwrap();
        session
            .fail_sound(
                &crate::banner::SoundCue::Default(id(9)),
                crate::banner::SoundPlaybackError::Busy,
                rmac_notifications::Time(1),
            )
            .unwrap();
        let feedback = session.accessibility_snapshot(&[]).unwrap();
        assert_eq!(feedback.feedback.len(), 1);
        assert_eq!(feedback.feedback[0].role, AccessibleRole::Status);
        assert_eq!(feedback.feedback[0].live_region, LiveRegion::Polite);
        assert_eq!(
            feedback.feedback[0].dismiss_action.kind,
            AccessibleActionKind::DismissFeedback
        );
        assert!(format!("{feedback:?}").contains("feedback_count: 1"));

        let mut cards = vec![card(id(1), output("private-output"), 0)];
        cards[0].body = "x".repeat(MAX_ACCESSIBLE_TEXT_VALUE_BYTES + 1);
        let banners = cards.iter().map(banner).collect();
        assert_eq!(
            project_banner_accessibility(&frame(&cards, banners), None, None, &[]),
            Err(AccessibilityProjectionError::TextValueLimit)
        );

        let cards = (1..=u32::try_from(MAX_CARDS + 1).unwrap())
            .map(|value| card(id(value), output("private-output"), value as usize - 1))
            .collect::<Vec<_>>();
        let banners = cards.iter().map(banner).collect();
        assert_eq!(
            project_banner_accessibility(&frame(&cards, banners), None, None, &[]),
            Err(AccessibilityProjectionError::CardLimit)
        );
    }
}
