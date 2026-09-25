//! Validated notification domain model.

use super::*;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NotificationId(pub(super) u32);

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
    pub(super) id: String,
    label: String,
    pub(super) target: Option<ActionTarget>,
    pub(super) purpose: Option<String>,
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
    pub(super) fn new(field: Field, problem: Problem) -> Self {
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
            normal_ms: 5_000,
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
    pub sounds: bool,
    pub history: HistoryPolicy,
    pub allow_urgent_through_focus: bool,
    pub focus_active: bool,
}

impl Default for DeliveryPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            banner: BannerPolicy::Allow,
            sounds: true,
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

/// A live notification the server closed to stay within its bounds. The
/// adapter reports it like an expiry: the banner goes, history is kept.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Eviction {
    pub closed: Closed,
    pub source: Source,
}

#[derive(Clone, Eq, PartialEq)]
pub struct ActionInvocation {
    pub notification_id: NotificationId,
    pub app_id: AppId,
    pub action_id: String,
    pub target: Option<ActionTarget>,
    pub purpose: Option<String>,
}

impl fmt::Debug for ActionInvocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActionInvocation")
            .field("notification_id", &self.notification_id)
            .field("app_id", &self.app_id)
            .field("action_id", &"<redacted>")
            .field("target", &self.target)
            .field("purpose", &self.purpose.as_ref().map(|_| "<redacted>"))
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
    /// Every live notification that could make room is urgent or persistent.
    TooManyNotifications,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Indicator {
    pub unread_count: u32,
    pub has_urgent: bool,
}

#[derive(Debug)]
pub struct Server {
    pub(super) next_id: u32,
    pub(super) reserved_ids: BTreeSet<NotificationId>,
    pub(super) timeout_policy: TimeoutPolicy,
    pub(super) history_limit: usize,
    pub(super) active: BTreeMap<NotificationId, Notification>,
    pub(super) history: VecDeque<Notification>,
    pub(super) evictions: Vec<Eviction>,
}
