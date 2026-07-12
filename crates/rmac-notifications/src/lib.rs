//! Framework-neutral notification validation, policy, and state transitions.
//!
//! D-Bus and XDG portal adapters normalize untrusted requests into [`Request`].
//! The reducer intentionally owns no bus connection, timer, persistence file,
//! sound player, or UI so every presentation surface observes one authority.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

pub mod banner;
pub mod protocol;

const MAX_APP_ID_BYTES: usize = 256;
const MAX_EXTERNAL_ID_BYTES: usize = 256;
const MAX_TITLE_BYTES: usize = 512;
const MAX_BODY_BYTES: usize = 16 * 1024;
const MAX_ACTIONS: usize = 8;
const MAX_ACTION_ID_BYTES: usize = 256;
const MAX_ACTION_LABEL_BYTES: usize = 256;
const MAX_TARGET_BYTES: usize = 16 * 1024;
const MAX_CATEGORY_BYTES: usize = 128;
const MAX_TIMEOUT_MS: u64 = 24 * 60 * 60 * 1_000;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NotificationId(u32);

impl NotificationId {
    pub fn from_protocol(value: u32) -> Option<Self> {
        (value != 0).then_some(Self(value))
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct AppId(String);

impl AppId {
    pub fn parse(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        validate_identifier(&value, MAX_APP_ID_BYTES, Field::AppId)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AppId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AppId(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
    Urgent,
}

impl Priority {
    /// Maps freedesktop urgency bytes: 0 low, 1 normal, 2 critical.
    pub fn from_freedesktop(value: u8) -> Result<Self, ValidationError> {
        match value {
            0 => Ok(Self::Low),
            1 => Ok(Self::Normal),
            2 => Ok(Self::Urgent),
            _ => Err(ValidationError::new(Field::Priority, Problem::Unsupported)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Sound {
    #[default]
    Policy,
    Default,
    Silent,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LockScreenVisibility {
    /// Defer to the user's per-app lock-screen policy.
    #[default]
    Policy,
    Show,
    HideContent,
    Hide,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DisplayHints {
    pub transient: bool,
    pub tray_only: bool,
    pub persistent: bool,
    /// Freedesktop `resident`: invoking an action does not close the record.
    pub resident: bool,
    pub show_as_new: bool,
    pub lock_screen: LockScreenVisibility,
}

#[derive(Clone, Eq, PartialEq)]
pub struct ActionTarget {
    /// D-Bus type signature supplied by the transport adapter.
    signature: String,
    /// Canonical transport-owned bytes. Never interpreted by the shell.
    bytes: Vec<u8>,
}

impl ActionTarget {
    pub fn new(signature: impl Into<String>, bytes: Vec<u8>) -> Result<Self, ValidationError> {
        let signature = signature.into();
        if signature.is_empty() || signature.len() > 64 || signature.chars().any(char::is_control) {
            return Err(ValidationError::new(Field::ActionTarget, Problem::Invalid));
        }
        if bytes.len() > MAX_TARGET_BYTES {
            return Err(ValidationError::new(Field::ActionTarget, Problem::TooLong));
        }
        Ok(Self { signature, bytes })
    }

    pub fn signature(&self) -> &str {
        &self.signature
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for ActionTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActionTarget")
            .field("signature", &self.signature)
            .field("bytes", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Action {
    id: String,
    label: String,
    target: Option<ActionTarget>,
    purpose: Option<String>,
}

impl Action {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        target: Option<ActionTarget>,
    ) -> Result<Self, ValidationError> {
        let id = id.into();
        let label = label.into();
        validate_identifier(&id, MAX_ACTION_ID_BYTES, Field::ActionId)?;
        validate_text(&label, MAX_ACTION_LABEL_BYTES, false, Field::ActionLabel)?;
        Ok(Self {
            id,
            label,
            target,
            purpose: None,
        })
    }

    pub fn with_purpose(mut self, purpose: impl Into<String>) -> Result<Self, ValidationError> {
        let purpose = purpose.into();
        validate_identifier(&purpose, MAX_CATEGORY_BYTES, Field::ActionPurpose)?;
        self.purpose = Some(purpose);
        Ok(self)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn target(&self) -> Option<&ActionTarget> {
        self.target.as_ref()
    }

    pub fn purpose(&self) -> Option<&str> {
        self.purpose.as_deref()
    }
}

impl fmt::Debug for Action {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Action")
            .field("id", &"<redacted>")
            .field("label", &"<redacted>")
            .field("target", &self.target)
            .field("purpose", &self.purpose.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Content {
    title: String,
    body: String,
}

impl Content {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Result<Self, ValidationError> {
        let title = title.into();
        let body = body.into();
        validate_text(&title, MAX_TITLE_BYTES, false, Field::Title)?;
        validate_text(&body, MAX_BODY_BYTES, true, Field::Body)?;
        Ok(Self { title, body })
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn body(&self) -> &str {
        &self.body
    }
}

impl fmt::Debug for Content {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Content(<redacted>)")
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum Source {
    /// XDG portal IDs replace only within their application namespace.
    Portal { app_id: AppId, external_id: String },
    /// The D-Bus adapter owns sender authentication and numeric ID routing.
    Freedesktop { app_id: AppId },
}

impl fmt::Debug for Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Portal { app_id, .. } => formatter
                .debug_struct("Portal")
                .field("app_id", app_id)
                .field("external_id", &"<redacted>")
                .finish(),
            Self::Freedesktop { app_id } => formatter
                .debug_struct("Freedesktop")
                .field("app_id", app_id)
                .finish(),
        }
    }
}

impl Source {
    pub fn portal(app_id: AppId, external_id: impl Into<String>) -> Result<Self, ValidationError> {
        let external_id = external_id.into();
        validate_identifier(&external_id, MAX_EXTERNAL_ID_BYTES, Field::ExternalId)?;
        Ok(Self::Portal {
            app_id,
            external_id,
        })
    }

    pub fn app_id(&self) -> &AppId {
        match self {
            Self::Portal { app_id, .. } | Self::Freedesktop { app_id } => app_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Timeout {
    /// Server policy chooses a duration based on priority.
    #[default]
    Default,
    Never,
    Milliseconds(u64),
}

impl Timeout {
    pub fn from_freedesktop(value: i32) -> Result<Self, ValidationError> {
        match value {
            -1 => Ok(Self::Default),
            0 => Ok(Self::Never),
            1.. => Ok(Self::Milliseconds(value as u64)),
            _ => Err(ValidationError::new(Field::Timeout, Problem::Invalid)),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Request {
    pub source: Source,
    pub content: Content,
    pub priority: Priority,
    pub timeout: Timeout,
    pub default_action: Option<Action>,
    pub actions: Vec<Action>,
    pub category: Option<String>,
    pub sound: Sound,
    pub display: DisplayHints,
    /// A legacy Notify replacement. Portal replacement is derived from Source.
    pub replaces: Option<NotificationId>,
}

impl fmt::Debug for Request {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Request")
            .field("source", &self.source)
            .field("content", &self.content)
            .field("priority", &self.priority)
            .field("timeout", &self.timeout)
            .field("default_action", &self.default_action)
            .field("actions", &self.actions)
            .field("category", &self.category.as_ref().map(|_| "<redacted>"))
            .field("sound", &self.sound)
            .field("display", &self.display)
            .field("replaces", &self.replaces)
            .finish()
    }
}

impl Request {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.actions.len() > MAX_ACTIONS {
            return Err(ValidationError::new(Field::Actions, Problem::TooMany));
        }
        if self.display.transient && self.display.tray_only {
            return Err(ValidationError::new(Field::DisplayHints, Problem::Conflict));
        }
        if let Some(category) = &self.category {
            validate_identifier(category, MAX_CATEGORY_BYTES, Field::Category)?;
        }
        if matches!(self.source, Source::Portal { .. }) && self.replaces.is_some() {
            return Err(ValidationError::new(Field::Replacement, Problem::Conflict));
        }
        if let Timeout::Milliseconds(value) = self.timeout {
            if value > MAX_TIMEOUT_MS {
                return Err(ValidationError::new(Field::Timeout, Problem::TooLong));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Field {
    AppId,
    ExternalId,
    Title,
    Body,
    Priority,
    Timeout,
    ActionId,
    ActionLabel,
    ActionTarget,
    ActionPurpose,
    Actions,
    Category,
    DisplayHints,
    Replacement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Problem {
    Empty,
    TooLong,
    TooMany,
    Invalid,
    Unsupported,
    Duplicate,
    Conflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidationError {
    pub field: Field,
    pub problem: Problem,
}

impl ValidationError {
    fn new(field: Field, problem: Problem) -> Self {
        Self { field, problem }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Time(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimeoutPolicy {
    pub low_ms: u64,
    pub normal_ms: u64,
    pub high_ms: u64,
}

impl Default for TimeoutPolicy {
    fn default() -> Self {
        Self {
            low_ms: 5_000,
            normal_ms: 7_000,
            high_ms: 10_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryPolicy {
    Allow,
    Transient,
    Block,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BannerPolicy {
    Allow,
    Suppress,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeliveryPolicy {
    pub enabled: bool,
    pub banner: BannerPolicy,
    pub history: HistoryPolicy,
    pub allow_urgent_through_focus: bool,
    pub focus_active: bool,
}

impl Default for DeliveryPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            banner: BannerPolicy::Allow,
            history: HistoryPolicy::Allow,
            allow_urgent_through_focus: true,
            focus_active: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Delivery {
    pub banner: bool,
    pub history: bool,
    pub sound: bool,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Notification {
    pub id: NotificationId,
    pub source: Source,
    pub content: Content,
    pub priority: Priority,
    pub default_action: Option<Action>,
    pub actions: Vec<Action>,
    pub category: Option<String>,
    pub sound: Sound,
    pub display: DisplayHints,
    pub delivery: Delivery,
    pub banner_visible: bool,
    pub created_at: Time,
    pub updated_at: Time,
    pub expires_at: Option<Time>,
    pub unread: bool,
}

impl fmt::Debug for Notification {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Notification")
            .field("id", &self.id)
            .field("source", &self.source)
            .field("content", &self.content)
            .field("priority", &self.priority)
            .field("delivery", &self.delivery)
            .field("banner_visible", &self.banner_visible)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("expires_at", &self.expires_at)
            .field("unread", &self.unread)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PostKind {
    Added,
    Replaced,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PostOutcome {
    pub id: NotificationId,
    pub kind: PostKind,
    pub delivery: Delivery,
    /// Replacement with `show-as-new` asks E2 to replay its entrance motion.
    pub announce_as_new: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseReason {
    Expired,
    Dismissed,
    Withdrawn,
    ActionInvoked,
    ReplacedByPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Closed {
    pub id: NotificationId,
    pub reason: CloseReason,
}

#[derive(Clone, Eq, PartialEq)]
pub struct ActionInvocation {
    pub notification_id: NotificationId,
    pub app_id: AppId,
    pub action_id: String,
    pub target: Option<ActionTarget>,
}

impl fmt::Debug for ActionInvocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActionInvocation")
            .field("notification_id", &self.notification_id)
            .field("app_id", &self.app_id)
            .field("action_id", &"<redacted>")
            .field("target", &self.target)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerError {
    Invalid(ValidationError),
    ExhaustedIds,
    UnknownNotification,
    UnknownAction,
    WrongOwner,
    PersistentNotification,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Indicator {
    pub unread_count: u32,
    pub has_urgent: bool,
}

#[derive(Debug)]
pub struct Server {
    next_id: u32,
    timeout_policy: TimeoutPolicy,
    history_limit: usize,
    active: BTreeMap<NotificationId, Notification>,
    history: VecDeque<Notification>,
}

impl Server {
    pub fn new(history_limit: usize, timeout_policy: TimeoutPolicy) -> Self {
        Self {
            next_id: 1,
            timeout_policy,
            history_limit,
            active: BTreeMap::new(),
            history: VecDeque::new(),
        }
    }

    pub fn post(
        &mut self,
        request: Request,
        now: Time,
        policy: DeliveryPolicy,
    ) -> Result<PostOutcome, ServerError> {
        request.validate().map_err(ServerError::Invalid)?;
        let replacement = self.replacement_id(&request)?;
        let (id, kind, created_at) = if let Some(id) = replacement {
            let previous = self
                .active
                .get(&id)
                .ok_or(ServerError::UnknownNotification)?;
            if previous.source.app_id() != request.source.app_id() {
                return Err(ServerError::WrongOwner);
            }
            (id, PostKind::Replaced, previous.created_at)
        } else {
            (self.allocate_id()?, PostKind::Added, now)
        };
        let delivery = delivery_for(&request, policy);
        let expires_at = expiry_for(request.timeout, request.priority, now, self.timeout_policy);
        let announce_as_new = kind == PostKind::Added || request.display.show_as_new;
        let notification = Notification {
            id,
            source: request.source,
            content: request.content,
            priority: request.priority,
            default_action: request.default_action,
            actions: request.actions,
            category: request.category,
            sound: request.sound,
            display: request.display,
            delivery,
            banner_visible: delivery.banner,
            created_at,
            updated_at: now,
            expires_at,
            unread: delivery.history,
        };
        self.remove_history(id);
        if delivery.history {
            self.push_history(notification.clone());
        }
        if policy.enabled {
            self.active.insert(id, notification);
        } else {
            self.active.remove(&id);
        }
        Ok(PostOutcome {
            id,
            kind,
            delivery,
            announce_as_new,
        })
    }

    pub fn withdraw(&mut self, app_id: &AppId, id: NotificationId) -> Result<Closed, ServerError> {
        let notification = self
            .active
            .get(&id)
            .ok_or(ServerError::UnknownNotification)?;
        if notification.source.app_id() != app_id {
            return Err(ServerError::WrongOwner);
        }
        self.active.remove(&id);
        self.remove_history(id);
        Ok(Closed {
            id,
            reason: CloseReason::Withdrawn,
        })
    }

    pub fn withdraw_portal(
        &mut self,
        app_id: &AppId,
        external_id: &str,
    ) -> Result<Closed, ServerError> {
        let id = self
            .active
            .iter()
            .find_map(|(id, notification)| match &notification.source {
                Source::Portal {
                    app_id: owner,
                    external_id: candidate,
                } if owner == app_id && candidate == external_id => Some(*id),
                _ => None,
            })
            .ok_or(ServerError::UnknownNotification)?;
        self.withdraw(app_id, id)
    }

    pub fn dismiss(&mut self, id: NotificationId) -> Result<Closed, ServerError> {
        let notification = self
            .active
            .get(&id)
            .ok_or(ServerError::UnknownNotification)?;
        if notification.display.persistent {
            return Err(ServerError::PersistentNotification);
        }
        self.active.remove(&id);
        self.remove_history(id);
        Ok(Closed {
            id,
            reason: CloseReason::Dismissed,
        })
    }

    pub fn expire(&mut self, now: Time) -> Vec<Closed> {
        let expired: Vec<_> = self
            .active
            .iter()
            .filter_map(|(id, notification)| {
                notification
                    .banner_visible
                    .then_some(notification.expires_at)
                    .flatten()
                    .filter(|expiry| expiry.0 <= now.0)
                    .map(|_| *id)
            })
            .collect();
        for id in &expired {
            let retain = self
                .active
                .get(id)
                .is_some_and(|notification| notification.delivery.history);
            if retain {
                if let Some(notification) = self.active.get_mut(id) {
                    notification.banner_visible = false;
                    notification.expires_at = None;
                }
            } else {
                self.active.remove(id);
            }
        }
        expired
            .into_iter()
            .map(|id| Closed {
                id,
                reason: CloseReason::Expired,
            })
            .collect()
    }

    pub fn invoke(
        &mut self,
        id: NotificationId,
        action_id: &str,
    ) -> Result<(ActionInvocation, Option<Closed>), ServerError> {
        let notification = self
            .active
            .get(&id)
            .ok_or(ServerError::UnknownNotification)?;
        let index = notification
            .default_action
            .iter()
            .chain(notification.actions.iter())
            .position(|action| action.id() == action_id)
            .ok_or(ServerError::UnknownAction)?;
        self.invoke_index(id, index)
    }

    /// Activates the notification's default action, if declared.
    pub fn invoke_default(
        &mut self,
        id: NotificationId,
    ) -> Result<(ActionInvocation, Option<Closed>), ServerError> {
        if self
            .active
            .get(&id)
            .ok_or(ServerError::UnknownNotification)?
            .default_action
            .is_none()
        {
            return Err(ServerError::UnknownAction);
        }
        self.invoke_index(id, 0)
    }

    /// Activates a portal button by its declared position. This preserves the
    /// target when buttons intentionally export the same action name.
    pub fn invoke_button(
        &mut self,
        id: NotificationId,
        button_index: usize,
    ) -> Result<(ActionInvocation, Option<Closed>), ServerError> {
        let notification = self
            .active
            .get(&id)
            .ok_or(ServerError::UnknownNotification)?;
        if button_index >= notification.actions.len() {
            return Err(ServerError::UnknownAction);
        }
        let index = usize::from(notification.default_action.is_some()) + button_index;
        self.invoke_index(id, index)
    }

    fn invoke_index(
        &mut self,
        id: NotificationId,
        index: usize,
    ) -> Result<(ActionInvocation, Option<Closed>), ServerError> {
        let notification = self
            .active
            .get(&id)
            .ok_or(ServerError::UnknownNotification)?;
        let action = notification
            .default_action
            .iter()
            .chain(notification.actions.iter())
            .nth(index)
            .ok_or(ServerError::UnknownAction)?;
        let invocation = ActionInvocation {
            notification_id: id,
            app_id: notification.source.app_id().clone(),
            action_id: action.id.clone(),
            target: action.target.clone(),
        };
        let closed = if notification.display.resident {
            None
        } else {
            self.active.remove(&id);
            self.remove_history(id);
            Some(Closed {
                id,
                reason: CloseReason::ActionInvoked,
            })
        };
        Ok((invocation, closed))
    }

    pub fn mark_all_read(&mut self) {
        for notification in &mut self.history {
            notification.unread = false;
        }
        for notification in self.active.values_mut() {
            notification.unread = false;
        }
    }

    pub fn clear_history(&mut self, app_id: Option<&AppId>) {
        self.history.retain(|notification| {
            app_id.is_some_and(|app_id| notification.source.app_id() != app_id)
        });
    }

    pub fn active(&self) -> impl Iterator<Item = &Notification> {
        self.active.values()
    }

    pub fn history(&self) -> impl DoubleEndedIterator<Item = &Notification> {
        self.history.iter()
    }

    pub fn indicator(&self) -> Indicator {
        let unread = self
            .history
            .iter()
            .filter(|notification| notification.unread);
        Indicator {
            unread_count: unread.clone().count().try_into().unwrap_or(u32::MAX),
            has_urgent: unread
                .into_iter()
                .any(|notification| notification.priority == Priority::Urgent),
        }
    }

    fn replacement_id(&self, request: &Request) -> Result<Option<NotificationId>, ServerError> {
        if let Some(id) = request.replaces {
            return Ok(Some(id));
        }
        let Source::Portal {
            app_id,
            external_id,
        } = &request.source
        else {
            return Ok(None);
        };
        Ok(self
            .active
            .iter()
            .find_map(|(id, notification)| match &notification.source {
                Source::Portal {
                    app_id: owner,
                    external_id: candidate,
                } if owner == app_id && candidate == external_id => Some(*id),
                _ => None,
            }))
    }

    fn allocate_id(&mut self) -> Result<NotificationId, ServerError> {
        for _ in 0..u32::MAX {
            let candidate = self.next_id.max(1);
            self.next_id = candidate.wrapping_add(1).max(1);
            let id = NotificationId(candidate);
            if !self.active.contains_key(&id) {
                return Ok(id);
            }
        }
        Err(ServerError::ExhaustedIds)
    }

    fn push_history(&mut self, notification: Notification) {
        if self.history_limit == 0 {
            return;
        }
        self.history.push_back(notification);
        while self.history.len() > self.history_limit {
            self.history.pop_front();
        }
    }

    fn remove_history(&mut self, id: NotificationId) {
        self.history.retain(|notification| notification.id != id);
    }
}

fn delivery_for(request: &Request, policy: DeliveryPolicy) -> Delivery {
    if !policy.enabled {
        return Delivery {
            banner: false,
            history: false,
            sound: false,
        };
    }
    let focus_allows = !policy.focus_active
        || (policy.allow_urgent_through_focus && request.priority == Priority::Urgent);
    let banner = !request.display.tray_only && policy.banner == BannerPolicy::Allow && focus_allows;
    let history = !request.display.transient && policy.history == HistoryPolicy::Allow;
    let sound = banner && request.sound != Sound::Silent;
    Delivery {
        banner,
        history,
        sound,
    }
}

fn expiry_for(
    timeout: Timeout,
    priority: Priority,
    now: Time,
    policy: TimeoutPolicy,
) -> Option<Time> {
    let duration = match timeout {
        Timeout::Never => return None,
        Timeout::Milliseconds(value) => value,
        Timeout::Default => match priority {
            Priority::Low => policy.low_ms,
            Priority::Normal => policy.normal_ms,
            Priority::High => policy.high_ms,
            Priority::Urgent => return None,
        },
    };
    Some(Time(now.0.saturating_add(duration)))
}

fn validate_identifier(value: &str, max: usize, field: Field) -> Result<(), ValidationError> {
    if value.trim().is_empty() {
        return Err(ValidationError::new(field, Problem::Empty));
    }
    if value.len() > max {
        return Err(ValidationError::new(field, Problem::TooLong));
    }
    if value.chars().any(char::is_control) {
        return Err(ValidationError::new(field, Problem::Invalid));
    }
    Ok(())
}

fn validate_text(
    value: &str,
    max: usize,
    multiline: bool,
    field: Field,
) -> Result<(), ValidationError> {
    if value.len() > max {
        return Err(ValidationError::new(field, Problem::TooLong));
    }
    if value
        .chars()
        .any(|character| character.is_control() && !(multiline && matches!(character, '\n' | '\t')))
    {
        return Err(ValidationError::new(field, Problem::Invalid));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portal_request(app: &str, external: &str, title: &str) -> Request {
        Request {
            source: Source::portal(AppId::parse(app).unwrap(), external).unwrap(),
            content: Content::new(title, "Private message body").unwrap(),
            priority: Priority::Normal,
            timeout: Timeout::Default,
            default_action: Some(Action::new("app.open", "Open", None).unwrap()),
            actions: vec![Action::new("app.reply", "Reply", None).unwrap()],
            category: Some("im.received".into()),
            sound: Sound::Policy,
            display: DisplayHints::default(),
            replaces: None,
        }
    }

    #[test]
    fn portal_replacement_is_atomic_stable_and_app_scoped() {
        let mut server = Server::new(20, TimeoutPolicy::default());
        let first = server
            .post(
                portal_request("org.example.Chat", "message-4", "First"),
                Time(100),
                DeliveryPolicy::default(),
            )
            .unwrap();
        let replaced = server
            .post(
                portal_request("org.example.Chat", "message-4", "Updated"),
                Time(200),
                DeliveryPolicy::default(),
            )
            .unwrap();
        let other_app = server
            .post(
                portal_request("org.example.Mail", "message-4", "Mail"),
                Time(300),
                DeliveryPolicy::default(),
            )
            .unwrap();

        assert_eq!(replaced.id, first.id);
        assert_eq!(replaced.kind, PostKind::Replaced);
        assert!(!replaced.announce_as_new);
        assert_ne!(other_app.id, first.id);
        assert_eq!(server.active.get(&first.id).unwrap().created_at, Time(100));
        assert_eq!(
            server.active.get(&first.id).unwrap().content.title(),
            "Updated"
        );
        assert_eq!(server.history.len(), 2);
    }

    #[test]
    fn legacy_replacement_checks_owner_and_preserves_numeric_id() {
        let mut server = Server::new(10, TimeoutPolicy::default());
        let mut original = portal_request("org.example.Chat", "one", "First");
        original.source = Source::Freedesktop {
            app_id: AppId::parse("org.example.Chat").unwrap(),
        };
        let first = server
            .post(original, Time(1), DeliveryPolicy::default())
            .unwrap();

        let mut replacement = portal_request("org.example.Chat", "unused", "Second");
        replacement.source = Source::Freedesktop {
            app_id: AppId::parse("org.example.Chat").unwrap(),
        };
        replacement.replaces = Some(first.id);
        assert_eq!(
            server
                .post(replacement.clone(), Time(2), DeliveryPolicy::default())
                .unwrap()
                .id,
            first.id
        );
        replacement.source = Source::Freedesktop {
            app_id: AppId::parse("org.attacker.App").unwrap(),
        };
        assert_eq!(
            server.post(replacement, Time(3), DeliveryPolicy::default()),
            Err(ServerError::WrongOwner)
        );
    }

    #[test]
    fn focus_suppresses_banner_and_sound_but_retains_allowed_history() {
        let mut server = Server::new(10, TimeoutPolicy::default());
        let focused = DeliveryPolicy {
            focus_active: true,
            ..DeliveryPolicy::default()
        };
        let normal = server
            .post(
                portal_request("org.example.Chat", "one", "Message"),
                Time(0),
                focused,
            )
            .unwrap();
        assert_eq!(
            normal.delivery,
            Delivery {
                banner: false,
                history: true,
                sound: false
            }
        );

        let mut urgent = portal_request("org.example.Chat", "two", "Alarm");
        urgent.priority = Priority::Urgent;
        let urgent = server.post(urgent, Time(0), focused).unwrap();
        assert!(urgent.delivery.banner);
        assert!(urgent.delivery.sound);
    }

    #[test]
    fn transient_tray_and_block_policies_are_enforced() {
        let mut server = Server::new(10, TimeoutPolicy::default());
        let mut transient = portal_request("org.example.Chat", "one", "Transient");
        transient.display.transient = true;
        let result = server
            .post(transient, Time(0), DeliveryPolicy::default())
            .unwrap();
        assert_eq!(
            result.delivery,
            Delivery {
                banner: true,
                history: false,
                sound: true
            }
        );

        let no_history = DeliveryPolicy {
            history: HistoryPolicy::Block,
            ..DeliveryPolicy::default()
        };
        let result = server
            .post(
                portal_request("org.example.Chat", "two", "No history"),
                Time(1),
                no_history,
            )
            .unwrap();
        assert_eq!(
            result.delivery,
            Delivery {
                banner: true,
                history: false,
                sound: true
            }
        );

        let blocked = DeliveryPolicy {
            enabled: false,
            ..DeliveryPolicy::default()
        };
        let result = server
            .post(
                portal_request("org.example.Chat", "three", "Blocked"),
                Time(2),
                blocked,
            )
            .unwrap();
        assert_eq!(
            result.delivery,
            Delivery {
                banner: false,
                history: false,
                sound: false
            }
        );
        assert_eq!(server.active().count(), 2);
    }

    #[test]
    fn default_expiry_respects_priority_and_explicit_protocol_values() {
        let policy = TimeoutPolicy::default();
        assert_eq!(
            expiry_for(Timeout::Default, Priority::Normal, Time(10), policy),
            Some(Time(7_010))
        );
        assert_eq!(
            expiry_for(Timeout::Default, Priority::Urgent, Time(10), policy),
            None
        );
        assert_eq!(Timeout::from_freedesktop(-1).unwrap(), Timeout::Default);
        assert_eq!(Timeout::from_freedesktop(0).unwrap(), Timeout::Never);
        assert_eq!(
            Timeout::from_freedesktop(250).unwrap(),
            Timeout::Milliseconds(250)
        );
        assert!(Timeout::from_freedesktop(-2).is_err());
    }

    #[test]
    fn expiration_leaves_history_and_actions_close_unless_persistent() {
        let mut server = Server::new(10, TimeoutPolicy::default());
        let posted = server
            .post(
                portal_request("org.example.Chat", "one", "Message"),
                Time(0),
                DeliveryPolicy::default(),
            )
            .unwrap();
        assert!(server.expire(Time(6_999)).is_empty());
        assert_eq!(
            server.expire(Time(7_000)),
            vec![Closed {
                id: posted.id,
                reason: CloseReason::Expired
            }]
        );
        assert_eq!(server.history().count(), 1);

        let second = server
            .post(
                portal_request("org.example.Chat", "two", "Reply"),
                Time(8_000),
                DeliveryPolicy::default(),
            )
            .unwrap();
        let (invocation, closed) = server.invoke(second.id, "app.reply").unwrap();
        assert_eq!(invocation.action_id, "app.reply");
        assert_eq!(closed.unwrap().reason, CloseReason::ActionInvoked);
        assert_eq!(server.history().count(), 1);
    }

    #[test]
    fn persistent_notification_rejects_user_dismissal_but_allows_sender_withdrawal() {
        let mut server = Server::new(10, TimeoutPolicy::default());
        let mut request = portal_request("org.example.Chat", "one", "Ongoing call");
        request.display.persistent = true;
        let app_id = request.source.app_id().clone();
        let posted = server
            .post(request, Time(0), DeliveryPolicy::default())
            .unwrap();
        assert_eq!(
            server.dismiss(posted.id),
            Err(ServerError::PersistentNotification)
        );
        assert_eq!(
            server.withdraw(&app_id, posted.id).unwrap().reason,
            CloseReason::Withdrawn
        );
    }

    #[test]
    fn validation_rejects_ambiguous_or_unbounded_input() {
        let mut request = portal_request("org.example.Chat", "one", "Message");
        for index in 0..MAX_ACTIONS {
            request
                .actions
                .push(Action::new(format!("app.extra-{index}"), "Extra", None).unwrap());
        }
        assert_eq!(request.validate().unwrap_err().problem, Problem::TooMany);
        request.actions.truncate(1);
        request.display.transient = true;
        request.display.tray_only = true;
        assert_eq!(request.validate().unwrap_err().problem, Problem::Conflict);
        assert!(Content::new("ok", "x".repeat(MAX_BODY_BYTES + 1)).is_err());
    }

    #[test]
    fn debug_output_redacts_user_content_and_action_targets() {
        let target = ActionTarget::new("s", b"secret target".to_vec()).unwrap();
        let mut request = portal_request(
            "org.example.SecretChat8472",
            "secret-external-8472",
            "Secret title",
        );
        request.content = Content::new("Secret title", "Secret body").unwrap();
        request.actions = vec![Action::new("app.reply", "Secret label", Some(target)).unwrap()];
        let debug = format!("{request:?}");
        assert!(!debug.contains("Secret title"));
        assert!(!debug.contains("Secret body"));
        assert!(!debug.contains("Secret label"));
        assert!(!debug.contains("secret target"));
        assert!(!debug.contains("org.example.SecretChat8472"));
        assert!(!debug.contains("secret-external-8472"));
    }

    #[test]
    fn repeated_portal_actions_keep_the_selected_target() {
        let mut server = Server::new(10, TimeoutPolicy::default());
        let mut request = portal_request("org.example.Chat", "one", "Message");
        request.actions = vec![
            Action::new(
                "app.open",
                "First",
                Some(ActionTarget::new("s", b"first".to_vec()).unwrap()),
            )
            .unwrap(),
            Action::new(
                "app.open",
                "Second",
                Some(ActionTarget::new("s", b"second".to_vec()).unwrap()),
            )
            .unwrap(),
        ];
        let posted = server
            .post(request, Time(0), DeliveryPolicy::default())
            .unwrap();
        let (invocation, _) = server.invoke_button(posted.id, 1).unwrap();
        assert_eq!(invocation.target.unwrap().bytes(), b"second");
    }

    #[test]
    fn bounded_history_and_indicator_are_deterministic() {
        let mut server = Server::new(2, TimeoutPolicy::default());
        for index in 0..3 {
            let mut request = portal_request("org.example.Chat", &format!("id-{index}"), "Message");
            if index == 2 {
                request.priority = Priority::Urgent;
            }
            server
                .post(request, Time(index), DeliveryPolicy::default())
                .unwrap();
        }
        assert_eq!(server.history().count(), 2);
        assert_eq!(
            server.indicator(),
            Indicator {
                unread_count: 2,
                has_urgent: true
            }
        );
        server.mark_all_read();
        assert_eq!(server.indicator(), Indicator::default());
    }
}
