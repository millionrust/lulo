//! Bounded panel, group, record, action, focus-order, and live-region semantics.

use std::collections::{HashMap, HashSet};
use std::fmt;

use rmac_notifications::{NotificationId, Priority};
use rmac_notifications_linux::center::{ActionSelection, HistoryRecord, Snapshot};

pub const PANEL_TITLE: &str = "Notification Center";
pub const CLEAR_ALL_LABEL: &str = "Clear All";
pub const TURN_OFF_LABEL: &str = "Turn Off";
pub const CLEAR_LABEL: &str = "Clear";
pub const URGENT_LABEL: &str = "Urgent";
pub const LOADING_LABEL: &str = "Loading Notification Center…";
pub const UNAVAILABLE_TITLE: &str = "Notification Center Unavailable";
pub const UNAVAILABLE_MESSAGE: &str = "Use Refresh after the notification service starts";
pub const EMPTY_TITLE: &str = "No recent notifications";
pub const EMPTY_MESSAGE: &str = "Notifications you keep will appear here";
pub const REFRESH_LABEL: &str = "Refresh";
pub const EDIT_WIDGETS_LABEL: &str = "Edit Widgets";
pub const SHOW_LESS_LABEL: &str = "Show Less";
pub const SETTINGS_LABEL: &str = "Notification Settings…";
pub const MARKING_READ_LABEL: &str = "Marking notifications as read…";
pub const NOTIFICATION_FALLBACK_NAME: &str = "Notification";
pub const MAX_ACCESSIBLE_RECORDS: usize = 500;
pub const MAX_ACCESSIBLE_APPLICATIONS: usize = 1_012;
pub const MAX_ACCESSIBLE_ACTIONS_PER_RECORD: usize = 9;
pub const MAX_ACCESSIBLE_APP_NAME_BYTES: usize = 4 * 1024;
pub const MAX_ACCESSIBLE_ACTION_NAME_BYTES: usize = 4 * 1024;
pub const MAX_ACCESSIBLE_TEXT_VALUE_BYTES: usize = 16 * 1024;
pub const MAX_ACCESSIBLE_TEXT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelState {
    Loading,
    Unavailable,
    Empty,
    Content,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionKind {
    ClearAll,
    TurnOffApplication,
    ClearApplication,
    InvokeDefault,
    InvokeButton(u8),
    Refresh,
    RouteToSettings,
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleAction {
    pub id: String,
    pub name: String,
    pub kind: ActionKind,
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
            .field("enabled", &self.enabled)
            .field("busy", &self.busy)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleNotificationRecord {
    pub id: NotificationId,
    pub name: String,
    pub title: String,
    pub body: String,
    pub unread: bool,
    pub urgent: bool,
    pub position_in_set: usize,
    pub set_size: usize,
    pub actions: Vec<AccessibleAction>,
}

impl fmt::Debug for AccessibleNotificationRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleNotificationRecord")
            .field("id", &self.id)
            .field("name", &"<redacted>")
            .field("title", &"<redacted>")
            .field("body", &"<redacted>")
            .field("unread", &self.unread)
            .field("urgent", &self.urgent)
            .field("position_in_set", &self.position_in_set)
            .field("set_size", &self.set_size)
            .field("action_count", &self.actions.len())
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleApplicationGroup {
    pub name: String,
    pub notification_count: usize,
    pub count_label: String,
    pub position_in_set: usize,
    pub set_size: usize,
    pub turn_off_action: Option<AccessibleAction>,
    pub clear_action: AccessibleAction,
    pub records: Vec<AccessibleNotificationRecord>,
}

impl fmt::Debug for AccessibleApplicationGroup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleApplicationGroup")
            .field("name", &"<redacted>")
            .field("notification_count", &self.notification_count)
            .field("position_in_set", &self.position_in_set)
            .field("set_size", &self.set_size)
            .field("has_turn_off_action", &self.turn_off_action.is_some())
            .field("record_count", &self.records.len())
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessibleEmptyState {
    pub title: &'static str,
    pub message: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LivePoliteness {
    Polite,
    Assertive,
}

#[derive(Clone, Eq, PartialEq)]
pub struct LiveAnnouncement {
    pub id: String,
    pub text: String,
    pub politeness: LivePoliteness,
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

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum PanelBusy<'a> {
    ClearAll,
    ClearApplication(&'a str),
    DisableApplication(&'a str),
    Invoke {
        notification: NotificationId,
        selection: ActionSelection,
    },
}

impl fmt::Debug for PanelBusy<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ClearAll => "PanelBusy::ClearAll",
            Self::ClearApplication(_) => "PanelBusy::ClearApplication(<redacted>)",
            Self::DisableApplication(_) => "PanelBusy::DisableApplication(<redacted>)",
            Self::Invoke { .. } => "PanelBusy::Invoke(<redacted>)",
        })
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct PanelStatus<'a> {
    pub stream_error: Option<&'a str>,
    pub operation_error: Option<&'a str>,
    pub busy: Option<PanelBusy<'a>>,
    pub marking_read: bool,
}

impl fmt::Debug for PanelStatus<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PanelStatus")
            .field("has_stream_error", &self.stream_error.is_some())
            .field("has_operation_error", &self.operation_error.is_some())
            .field("busy", &self.busy)
            .field("marking_read", &self.marking_read)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeaderText<'a> {
    pub time: &'a str,
    pub date: &'a str,
}

#[derive(Clone, Eq, PartialEq)]
pub struct NotificationCenterAccessibilitySnapshot {
    pub title: &'static str,
    pub time: String,
    pub date: String,
    pub state: PanelState,
    pub groups: Vec<AccessibleApplicationGroup>,
    pub empty_state: Option<AccessibleEmptyState>,
    pub clear_all_action: Option<AccessibleAction>,
    pub refresh_action: AccessibleAction,
    pub settings_action: AccessibleAction,
    /// Enabled action identities in exact visual reading order.
    pub keyboard_order: Vec<String>,
    pub initial_focus: String,
    pub announcements: Vec<LiveAnnouncement>,
}

impl fmt::Debug for NotificationCenterAccessibilitySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NotificationCenterAccessibilitySnapshot")
            .field("state", &self.state)
            .field("group_count", &self.groups.len())
            .field(
                "record_count",
                &self
                    .groups
                    .iter()
                    .map(|group| group.records.len())
                    .sum::<usize>(),
            )
            .field("keyboard_action_count", &self.keyboard_order.len())
            .field("initial_focus", &self.initial_focus)
            .field("announcement_count", &self.announcements.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibilityProjectionError {
    RecordLimit,
    ApplicationLimit,
    ActionLimit,
    DuplicateApplication,
    DuplicateNotification,
    DuplicateAction,
    MissingApplicationName,
    InvalidActionState,
    InvalidText,
    TextValueLimit,
    TextLimit,
}

/// Projects the already validated service snapshot without repeating history,
/// grouping, policy, or action authority. `resolve_application_name` is the
/// panel's exact catalog/fallback identity resolver; no icon or source path
/// crosses this boundary.
pub fn project_notification_center<F>(
    snapshot: Option<&Snapshot>,
    header: HeaderText<'_>,
    status: PanelStatus<'_>,
    mut resolve_application_name: F,
) -> Result<NotificationCenterAccessibilitySnapshot, AccessibilityProjectionError>
where
    F: FnMut(&str) -> String,
{
    let mut budget = TextBudget::default();
    budget.add_required(PANEL_TITLE, MAX_ACCESSIBLE_TEXT_VALUE_BYTES, false)?;
    budget.add_required(header.time, MAX_ACCESSIBLE_TEXT_VALUE_BYTES, false)?;
    budget.add_required(header.date, MAX_ACCESSIBLE_TEXT_VALUE_BYTES, false)?;

    let state = match (snapshot, status.stream_error) {
        (None, None) => PanelState::Loading,
        (None, Some(_)) => PanelState::Unavailable,
        (Some(snapshot), _) if snapshot.records.is_empty() => PanelState::Empty,
        (Some(_), _) => PanelState::Content,
    };

    let mut keyboard_order = Vec::new();
    let mut groups = Vec::new();
    let mut clear_all_action = None;
    if let Some(snapshot) = snapshot {
        validate_snapshot_bounds(snapshot)?;
        let policies = application_policies(snapshot)?;
        let raw_groups = grouped_records(snapshot)?;
        let group_count = raw_groups.len();
        let content_enabled = status.busy.is_none();

        if !snapshot.records.is_empty() {
            let action = accessible_action(
                "clear-all-notifications",
                CLEAR_ALL_LABEL,
                ActionKind::ClearAll,
                content_enabled,
                matches!(status.busy, Some(PanelBusy::ClearAll)),
                &mut budget,
            )?;
            push_enabled(&action, &mut keyboard_order);
            clear_all_action = Some(action);
        }

        for (group_index, (app_id, records)) in raw_groups.into_iter().enumerate() {
            let name = resolve_application_name(app_id);
            budget.add_required(&name, MAX_ACCESSIBLE_APP_NAME_BYTES, false)?;
            let policy_enabled = policies.get(app_id).copied().unwrap_or(true);
            let turn_off_action = policy_enabled
                .then(|| {
                    accessible_action(
                        format!("disable-group-{group_index}"),
                        TURN_OFF_LABEL,
                        ActionKind::TurnOffApplication,
                        content_enabled,
                        matches!(
                            status.busy,
                            Some(PanelBusy::DisableApplication(candidate)) if candidate == app_id
                        ),
                        &mut budget,
                    )
                })
                .transpose()?;
            if let Some(action) = &turn_off_action {
                push_enabled(action, &mut keyboard_order);
            }
            let clear_action = accessible_action(
                format!("clear-group-{group_index}"),
                CLEAR_LABEL,
                ActionKind::ClearApplication,
                content_enabled,
                matches!(
                    status.busy,
                    Some(PanelBusy::ClearApplication(candidate)) if candidate == app_id
                ),
                &mut budget,
            )?;
            push_enabled(&clear_action, &mut keyboard_order);

            let record_count = records.len();
            let count_label = notification_count_label(record_count);
            budget.add_required(&count_label, MAX_ACCESSIBLE_TEXT_VALUE_BYTES, false)?;
            let mut accessible_records = Vec::with_capacity(record_count);
            for (record_index, record) in records.into_iter().enumerate() {
                budget.add_optional(
                    record.content.title(),
                    MAX_ACCESSIBLE_TEXT_VALUE_BYTES,
                    false,
                )?;
                budget.add_optional(
                    record.content.body(),
                    MAX_ACCESSIBLE_TEXT_VALUE_BYTES,
                    true,
                )?;
                let mut record_actions = Vec::with_capacity(record.actions.len());
                for (action_index, action) in record.actions.iter().enumerate() {
                    let (kind, selection) = match action.selection {
                        ActionSelection::Default => {
                            (ActionKind::InvokeDefault, ActionSelection::Default)
                        }
                        ActionSelection::Button(index) => (
                            ActionKind::InvokeButton(index),
                            ActionSelection::Button(index),
                        ),
                    };
                    let accessible = accessible_action(
                        format!("notification-{}-action-{action_index}", record.id.get()),
                        &action.label,
                        kind,
                        content_enabled,
                        matches!(
                            status.busy,
                            Some(PanelBusy::Invoke {
                                notification,
                                selection: candidate,
                            }) if notification == record.id && candidate == selection
                        ),
                        &mut budget,
                    )?;
                    push_enabled(&accessible, &mut keyboard_order);
                    record_actions.push(accessible);
                }
                let name = if record.content.title().trim().is_empty() {
                    NOTIFICATION_FALLBACK_NAME.to_owned()
                } else {
                    record.content.title().to_owned()
                };
                budget.add_required(&name, MAX_ACCESSIBLE_TEXT_VALUE_BYTES, false)?;
                accessible_records.push(AccessibleNotificationRecord {
                    id: record.id,
                    name,
                    title: record.content.title().to_owned(),
                    body: record.content.body().to_owned(),
                    unread: record.unread,
                    urgent: record.priority == Priority::Urgent,
                    position_in_set: record_index + 1,
                    set_size: record_count,
                    actions: record_actions,
                });
            }

            groups.push(AccessibleApplicationGroup {
                name,
                notification_count: record_count,
                count_label,
                position_in_set: group_index + 1,
                set_size: group_count,
                turn_off_action,
                clear_action,
                records: accessible_records,
            });
        }
    }

    let refresh_action = accessible_action(
        "refresh-notifications",
        REFRESH_LABEL,
        ActionKind::Refresh,
        true,
        false,
        &mut budget,
    )?;
    push_enabled(&refresh_action, &mut keyboard_order);
    let settings_action = accessible_action(
        "open-notification-settings",
        SETTINGS_LABEL,
        ActionKind::RouteToSettings,
        true,
        false,
        &mut budget,
    )?;
    push_enabled(&settings_action, &mut keyboard_order);

    let mut announcements = Vec::new();
    match state {
        PanelState::Loading => push_announcement(
            &mut announcements,
            &mut budget,
            "notification-center-loading",
            LOADING_LABEL,
            LivePoliteness::Polite,
        )?,
        PanelState::Unavailable => push_announcement(
            &mut announcements,
            &mut budget,
            "notification-center-unavailable",
            status
                .stream_error
                .ok_or(AccessibilityProjectionError::InvalidText)?,
            LivePoliteness::Assertive,
        )?,
        PanelState::Empty => push_announcement(
            &mut announcements,
            &mut budget,
            "notification-center-empty",
            EMPTY_TITLE,
            LivePoliteness::Polite,
        )?,
        PanelState::Content => {}
    }
    if state != PanelState::Unavailable {
        if let Some(error) = status.stream_error {
            push_announcement(
                &mut announcements,
                &mut budget,
                "notification-center-stream-error",
                error,
                LivePoliteness::Assertive,
            )?;
        }
    }
    if let Some(error) = status.operation_error {
        push_announcement(
            &mut announcements,
            &mut budget,
            "notification-center-operation-error",
            error,
            LivePoliteness::Assertive,
        )?;
    }
    if status.marking_read {
        push_announcement(
            &mut announcements,
            &mut budget,
            "notification-center-marking-read",
            MARKING_READ_LABEL,
            LivePoliteness::Polite,
        )?;
    }
    if let Some(busy) = status.busy {
        let text = match busy {
            PanelBusy::ClearAll => "Clearing all notifications…",
            PanelBusy::ClearApplication(_) => "Clearing application notifications…",
            PanelBusy::DisableApplication(_) => "Turning off application notifications…",
            PanelBusy::Invoke { .. } => "Running notification action…",
        };
        push_announcement(
            &mut announcements,
            &mut budget,
            "notification-center-busy",
            text,
            LivePoliteness::Polite,
        )?;
    }

    let empty_state = match state {
        PanelState::Unavailable => Some(AccessibleEmptyState {
            title: UNAVAILABLE_TITLE,
            message: UNAVAILABLE_MESSAGE,
        }),
        PanelState::Empty => Some(AccessibleEmptyState {
            title: EMPTY_TITLE,
            message: EMPTY_MESSAGE,
        }),
        PanelState::Loading | PanelState::Content => None,
    };
    if let Some(empty) = &empty_state {
        budget.add_required(empty.title, MAX_ACCESSIBLE_TEXT_VALUE_BYTES, false)?;
        budget.add_required(empty.message, MAX_ACCESSIBLE_TEXT_VALUE_BYTES, false)?;
    }

    let initial_focus = keyboard_order
        .first()
        .cloned()
        .ok_or(AccessibilityProjectionError::InvalidActionState)?;
    validate_unique_actions(&keyboard_order)?;

    Ok(NotificationCenterAccessibilitySnapshot {
        title: PANEL_TITLE,
        time: header.time.to_owned(),
        date: header.date.to_owned(),
        state,
        groups,
        empty_state,
        clear_all_action,
        refresh_action,
        settings_action,
        keyboard_order,
        initial_focus,
        announcements,
    })
}

fn validate_snapshot_bounds(snapshot: &Snapshot) -> Result<(), AccessibilityProjectionError> {
    if snapshot.records.len() > MAX_ACCESSIBLE_RECORDS {
        return Err(AccessibilityProjectionError::RecordLimit);
    }
    if snapshot.applications.len() > MAX_ACCESSIBLE_APPLICATIONS {
        return Err(AccessibilityProjectionError::ApplicationLimit);
    }

    let mut notification_ids = HashSet::with_capacity(snapshot.records.len());
    for record in &snapshot.records {
        if !notification_ids.insert(record.id) {
            return Err(AccessibilityProjectionError::DuplicateNotification);
        }
        if record.actions.len() > MAX_ACCESSIBLE_ACTIONS_PER_RECORD {
            return Err(AccessibilityProjectionError::ActionLimit);
        }
        let mut selections = Vec::with_capacity(record.actions.len());
        for action in &record.actions {
            if selections.contains(&action.selection) {
                return Err(AccessibilityProjectionError::DuplicateAction);
            }
            selections.push(action.selection);
        }
    }
    Ok(())
}

fn application_policies(
    snapshot: &Snapshot,
) -> Result<HashMap<&str, bool>, AccessibilityProjectionError> {
    let mut policies = HashMap::with_capacity(snapshot.applications.len());
    for application in &snapshot.applications {
        if policies
            .insert(application.app_id.as_str(), application.policy.enabled)
            .is_some()
        {
            return Err(AccessibilityProjectionError::DuplicateApplication);
        }
    }
    Ok(policies)
}

fn grouped_records(
    snapshot: &Snapshot,
) -> Result<Vec<(&str, Vec<&HistoryRecord>)>, AccessibilityProjectionError> {
    let mut positions: HashMap<&str, usize> = HashMap::new();
    let mut groups: Vec<(&str, Vec<&HistoryRecord>)> = Vec::new();
    for record in &snapshot.records {
        if record.app_id.trim().is_empty() {
            return Err(AccessibilityProjectionError::MissingApplicationName);
        }
        if let Some(position) = positions.get(record.app_id.as_str()).copied() {
            groups[position].1.push(record);
        } else {
            positions.insert(record.app_id.as_str(), groups.len());
            groups.push((record.app_id.as_str(), vec![record]));
        }
    }
    Ok(groups)
}

pub fn notification_count_label(count: usize) -> String {
    format!("{count} notification{}", if count == 1 { "" } else { "s" })
}

fn accessible_action(
    id: impl Into<String>,
    name: &str,
    kind: ActionKind,
    enabled: bool,
    busy: bool,
    budget: &mut TextBudget,
) -> Result<AccessibleAction, AccessibilityProjectionError> {
    let id = id.into();
    budget.add_required(&id, 512, false)?;
    budget.add_required(name, MAX_ACCESSIBLE_ACTION_NAME_BYTES, false)?;
    if busy && enabled {
        return Err(AccessibilityProjectionError::InvalidActionState);
    }
    Ok(AccessibleAction {
        id,
        name: name.to_owned(),
        kind,
        enabled,
        busy,
    })
}

fn push_enabled(action: &AccessibleAction, order: &mut Vec<String>) {
    if action.enabled {
        order.push(action.id.clone());
    }
}

fn validate_unique_actions(order: &[String]) -> Result<(), AccessibilityProjectionError> {
    let mut seen = HashSet::with_capacity(order.len());
    if order.iter().any(|action| !seen.insert(action.as_str())) {
        return Err(AccessibilityProjectionError::DuplicateAction);
    }
    Ok(())
}

fn push_announcement(
    announcements: &mut Vec<LiveAnnouncement>,
    budget: &mut TextBudget,
    id: &str,
    text: &str,
    politeness: LivePoliteness,
) -> Result<(), AccessibilityProjectionError> {
    budget.add_required(id, 512, false)?;
    budget.add_required(text, MAX_ACCESSIBLE_TEXT_VALUE_BYTES, true)?;
    announcements.push(LiveAnnouncement {
        id: id.to_owned(),
        text: text.to_owned(),
        politeness,
    });
    Ok(())
}

#[derive(Default)]
struct TextBudget {
    bytes: usize,
}

impl TextBudget {
    fn add_required(
        &mut self,
        text: &str,
        max: usize,
        multiline: bool,
    ) -> Result<(), AccessibilityProjectionError> {
        if text.trim().is_empty() {
            return Err(AccessibilityProjectionError::InvalidText);
        }
        self.add_optional(text, max, multiline)
    }

    fn add_optional(
        &mut self,
        text: &str,
        max: usize,
        multiline: bool,
    ) -> Result<(), AccessibilityProjectionError> {
        if text.len() > max {
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
    use rmac_notifications::Content;
    use rmac_notifications_linux::center::{ApplicationPolicy, HistoryAction};
    use rmac_notifications_store::AppPolicy;

    fn record(
        id: u32,
        app_id: &str,
        title: &str,
        unread: bool,
        priority: Priority,
        actions: Vec<HistoryAction>,
    ) -> HistoryRecord {
        HistoryRecord {
            id: NotificationId::from_protocol(id).unwrap(),
            app_id: app_id.to_owned(),
            content: Content::new(title, format!("Body for {title}")).unwrap(),
            priority,
            unread,
            actions,
            origin: Default::default(),
        }
    }

    fn action(selection: ActionSelection, label: &str) -> HistoryAction {
        HistoryAction {
            selection,
            label: label.to_owned(),
        }
    }

    fn header() -> HeaderText<'static> {
        HeaderText {
            time: "10:42",
            date: "Sunday, August 2",
        }
    }

    fn ready() -> PanelStatus<'static> {
        PanelStatus {
            stream_error: None,
            operation_error: None,
            busy: None,
            marking_read: false,
        }
    }

    #[test]
    fn exact_groups_record_states_actions_and_keyboard_order_are_projected() {
        let snapshot = Snapshot {
            records: vec![
                record(
                    1,
                    "chat.desktop",
                    "First private title",
                    true,
                    Priority::Urgent,
                    vec![
                        action(ActionSelection::Default, "Open"),
                        action(ActionSelection::Button(2), "Reply"),
                    ],
                ),
                record(
                    2,
                    "chat.desktop",
                    "Second private title",
                    false,
                    Priority::Normal,
                    Vec::new(),
                ),
                record(
                    3,
                    "mail.desktop",
                    "Mail private title",
                    true,
                    Priority::High,
                    vec![action(ActionSelection::Default, "Read")],
                ),
            ],
            applications: vec![
                ApplicationPolicy {
                    app_id: "chat.desktop".into(),
                    policy: AppPolicy::default(),
                },
                ApplicationPolicy {
                    app_id: "mail.desktop".into(),
                    policy: AppPolicy::default(),
                },
            ],
        };

        let projected =
            project_notification_center(Some(&snapshot), header(), ready(), |id| match id {
                "chat.desktop" => "Chat".into(),
                "mail.desktop" => "Mail".into(),
                _ => String::new(),
            })
            .unwrap();

        assert_eq!(projected.state, PanelState::Content);
        assert_eq!(projected.groups.len(), 2);
        assert_eq!(projected.groups[0].name, "Chat");
        assert_eq!(projected.groups[0].notification_count, 2);
        assert!(projected.groups[0].records[0].unread);
        assert!(projected.groups[0].records[0].urgent);
        assert_eq!(projected.groups[0].records[0].position_in_set, 1);
        assert_eq!(projected.groups[0].records[0].set_size, 2);
        assert_eq!(
            projected.keyboard_order,
            vec![
                "clear-all-notifications",
                "disable-group-0",
                "clear-group-0",
                "notification-1-action-0",
                "notification-1-action-1",
                "disable-group-1",
                "clear-group-1",
                "notification-3-action-0",
                "refresh-notifications",
                "open-notification-settings",
            ]
        );
        assert_eq!(projected.initial_focus, "clear-all-notifications");
    }

    #[test]
    fn busy_errors_and_diagnostics_preserve_truth_without_private_text() {
        let snapshot = Snapshot {
            records: vec![record(
                7,
                "private.desktop",
                "Secret title",
                true,
                Priority::Urgent,
                vec![action(ActionSelection::Button(3), "Secret action")],
            )],
            applications: vec![ApplicationPolicy {
                app_id: "private.desktop".into(),
                policy: AppPolicy::default(),
            }],
        };
        let status = PanelStatus {
            stream_error: Some("Secret stream detail"),
            operation_error: Some("Secret operation detail"),
            busy: Some(PanelBusy::Invoke {
                notification: NotificationId::from_protocol(7).unwrap(),
                selection: ActionSelection::Button(3),
            }),
            marking_read: true,
        };

        let projected = project_notification_center(Some(&snapshot), header(), status, |_| {
            "Private Application".into()
        })
        .unwrap();
        let invocation = &projected.groups[0].records[0].actions[0];
        assert!(!invocation.enabled);
        assert!(invocation.busy);
        assert_eq!(
            projected.keyboard_order,
            vec!["refresh-notifications", "open-notification-settings"]
        );
        assert_eq!(projected.initial_focus, "refresh-notifications");
        assert!(projected
            .announcements
            .iter()
            .any(|announcement| announcement.politeness == LivePoliteness::Assertive));
        assert!(projected
            .announcements
            .iter()
            .any(|announcement| announcement.text == MARKING_READ_LABEL));

        let diagnostics = format!("{projected:?}");
        for private in [
            "Secret title",
            "Body for Secret title",
            "Secret action",
            "Private Application",
            "Secret stream detail",
            "Secret operation detail",
            "private.desktop",
        ] {
            assert!(!diagnostics.contains(private));
        }
        let input_diagnostics = format!(
            "{status:?} {:?}",
            PanelBusy::ClearApplication("private.desktop")
        );
        for private in [
            "Secret stream detail",
            "Secret operation detail",
            "private.desktop",
        ] {
            assert!(!input_diagnostics.contains(private));
        }
    }

    #[test]
    fn malformed_identity_and_duplicate_action_fail_closed_while_stale_busy_is_safe() {
        let duplicate_actions = Snapshot {
            records: vec![record(
                1,
                "app.desktop",
                "Title",
                false,
                Priority::Normal,
                vec![
                    action(ActionSelection::Button(1), "One"),
                    action(ActionSelection::Button(1), "Again"),
                ],
            )],
            applications: Vec::new(),
        };
        assert_eq!(
            project_notification_center(Some(&duplicate_actions), header(), ready(), |_| {
                "Application".into()
            }),
            Err(AccessibilityProjectionError::DuplicateAction)
        );

        let snapshot = Snapshot {
            records: vec![record(
                1,
                "app.desktop",
                "",
                false,
                Priority::Normal,
                Vec::new(),
            )],
            applications: Vec::new(),
        };
        assert_eq!(
            project_notification_center(Some(&snapshot), header(), ready(), |_| String::new()),
            Err(AccessibilityProjectionError::InvalidText)
        );
        let empty_title = project_notification_center(Some(&snapshot), header(), ready(), |_| {
            "Application".into()
        })
        .unwrap();
        assert_eq!(
            empty_title.groups[0].records[0].name,
            NOTIFICATION_FALLBACK_NAME
        );
        let stale_busy = project_notification_center(
            Some(&snapshot),
            header(),
            PanelStatus {
                busy: Some(PanelBusy::ClearApplication("gone.desktop")),
                ..ready()
            },
            |_| "Application".into(),
        )
        .unwrap();
        assert!(stale_busy
            .announcements
            .iter()
            .any(|announcement| announcement.id == "notification-center-busy"));
        assert_eq!(
            project_notification_center(Some(&snapshot), header(), ready(), |_| {
                "x".repeat(MAX_ACCESSIBLE_APP_NAME_BYTES + 1)
            }),
            Err(AccessibilityProjectionError::TextValueLimit)
        );
    }
}
