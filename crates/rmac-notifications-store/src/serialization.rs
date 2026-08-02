//! Versioned private notification history serialization.

use super::*;

#[derive(Deserialize, Serialize)]
pub(super) struct StoredFile {
    version: u32,
    history: Vec<StoredNotification>,
    policies: BTreeMap<String, AppPolicy>,
}

impl StoredFile {
    pub(super) fn from_center(center: &Center) -> Result<Self, Error> {
        Ok(Self {
            version: VERSION,
            history: center
                .history
                .iter()
                .map(StoredNotification::from_notification)
                .collect(),
            policies: center
                .policies
                .iter()
                .map(|(app_id, policy)| (app_id.as_str().to_owned(), *policy))
                .collect(),
        })
    }

    pub(super) fn into_center(self) -> Result<Center, Error> {
        if self.version != VERSION {
            return Err(Error::new(
                Operation::Validate,
                ErrorKind::UnsupportedVersion,
            ));
        }
        if self.history.len() > MAX_HISTORY || self.policies.len() > MAX_POLICIES {
            return Err(Error::new(Operation::Validate, ErrorKind::Limit));
        }
        let history = self
            .history
            .into_iter()
            .map(StoredNotification::into_notification)
            .collect::<Result<Vec<_>, _>>()?;
        let policies = self
            .policies
            .into_iter()
            .map(|(app_id, policy)| AppId::parse(app_id).map(|app_id| (app_id, policy)))
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid))?;
        let mut center = Center { history, policies };
        let app_ids: Vec<_> = center
            .history
            .iter()
            .map(|record| record.source.app_id().clone())
            .collect();
        for app_id in app_ids {
            center.enforce_bounds(&app_id);
        }
        Ok(center)
    }
}

#[derive(Deserialize, Serialize)]
pub(super) struct StoredNotification {
    id: u32,
    source: StoredSource,
    title: String,
    body: String,
    priority: StoredPriority,
    default_action: Option<StoredAction>,
    actions: Vec<StoredAction>,
    category: Option<String>,
    display: StoredDisplay,
    created_at: u64,
    updated_at: u64,
    unread: bool,
}

impl StoredNotification {
    fn from_notification(notification: &Notification) -> Self {
        Self {
            id: notification.id.get(),
            source: StoredSource::from_source(&notification.source),
            title: notification.content.title().to_owned(),
            body: notification.content.body().to_owned(),
            priority: notification.priority.into(),
            default_action: notification
                .default_action
                .as_ref()
                .map(StoredAction::from_action),
            actions: notification
                .actions
                .iter()
                .map(StoredAction::from_action)
                .collect(),
            category: notification.category.clone(),
            display: notification.display.into(),
            created_at: notification.created_at.0,
            updated_at: notification.updated_at.0,
            unread: notification.unread,
        }
    }

    fn into_notification(self) -> Result<Notification, Error> {
        let id = NotificationId::from_protocol(self.id)
            .ok_or_else(|| Error::new(Operation::Validate, ErrorKind::Invalid))?;
        let source = self.source.into_source()?;
        let content = Content::new(self.title, self.body)
            .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid))?;
        let default_action = self
            .default_action
            .map(StoredAction::into_action)
            .transpose()?;
        let actions = self
            .actions
            .into_iter()
            .map(StoredAction::into_action)
            .collect::<Result<Vec<_>, _>>()?;
        if actions.len() > 8 {
            return Err(Error::new(Operation::Validate, ErrorKind::Limit));
        }
        if self.created_at > self.updated_at
            || self.category.as_ref().is_some_and(|category| {
                category.trim().is_empty()
                    || category.len() > 128
                    || category.chars().any(char::is_control)
            })
        {
            return Err(Error::new(Operation::Validate, ErrorKind::Invalid));
        }
        Ok(Notification {
            id,
            source,
            content,
            priority: self.priority.into(),
            default_action,
            actions,
            category: self.category,
            sound: Sound::Silent,
            display: self.display.into(),
            delivery: Delivery {
                banner: false,
                history: true,
                sound: false,
            },
            banner_visible: false,
            created_at: Time(self.created_at),
            updated_at: Time(self.updated_at),
            expires_at: None,
            unread: self.unread,
        })
    }
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(super) enum StoredSource {
    Portal { app_id: String, external_id: String },
    Freedesktop { app_id: String },
}

impl StoredSource {
    fn from_source(source: &Source) -> Self {
        match source {
            Source::Portal {
                app_id,
                external_id,
            } => Self::Portal {
                app_id: app_id.as_str().to_owned(),
                external_id: external_id.clone(),
            },
            Source::Freedesktop { app_id } => Self::Freedesktop {
                app_id: app_id.as_str().to_owned(),
            },
        }
    }

    fn into_source(self) -> Result<Source, Error> {
        match self {
            Self::Portal {
                app_id,
                external_id,
            } => Source::portal(
                AppId::parse(app_id)
                    .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid))?,
                external_id,
            )
            .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid)),
            Self::Freedesktop { app_id } => Ok(Source::Freedesktop {
                app_id: AppId::parse(app_id)
                    .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid))?,
            }),
        }
    }
}

#[derive(Deserialize, Serialize)]
pub(super) struct StoredAction {
    id: String,
    label: String,
    target_signature: Option<String>,
    target_bytes: Option<Vec<u8>>,
    purpose: Option<String>,
}

impl StoredAction {
    fn from_action(action: &Action) -> Self {
        Self {
            id: action.id().to_owned(),
            label: action.label().to_owned(),
            target_signature: action.target().map(|target| target.signature().to_owned()),
            target_bytes: action.target().map(|target| target.bytes().to_vec()),
            purpose: action.purpose().map(str::to_owned),
        }
    }

    fn into_action(self) -> Result<Action, Error> {
        let target = match (self.target_signature, self.target_bytes) {
            (None, None) => None,
            (Some(signature), Some(bytes)) => Some(
                ActionTarget::new(signature, bytes)
                    .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid))?,
            ),
            _ => return Err(Error::new(Operation::Validate, ErrorKind::Invalid)),
        };
        let action = Action::new(self.id, self.label, target)
            .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid))?;
        match self.purpose {
            Some(purpose) => action
                .with_purpose(purpose)
                .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid)),
            None => Ok(action),
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum StoredPriority {
    Low,
    Normal,
    High,
    Urgent,
}

impl From<Priority> for StoredPriority {
    fn from(priority: Priority) -> Self {
        match priority {
            Priority::Low => Self::Low,
            Priority::Normal => Self::Normal,
            Priority::High => Self::High,
            Priority::Urgent => Self::Urgent,
        }
    }
}

impl From<StoredPriority> for Priority {
    fn from(priority: StoredPriority) -> Self {
        match priority {
            StoredPriority::Low => Self::Low,
            StoredPriority::Normal => Self::Normal,
            StoredPriority::High => Self::High,
            StoredPriority::Urgent => Self::Urgent,
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
pub(super) struct StoredDisplay {
    persistent: bool,
    resident: bool,
    lock_screen: StoredLockVisibility,
}

impl From<DisplayHints> for StoredDisplay {
    fn from(display: DisplayHints) -> Self {
        Self {
            persistent: display.persistent,
            resident: display.resident,
            lock_screen: display.lock_screen.into(),
        }
    }
}

impl From<StoredDisplay> for DisplayHints {
    fn from(display: StoredDisplay) -> Self {
        Self {
            persistent: display.persistent,
            resident: display.resident,
            lock_screen: display.lock_screen.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum StoredLockVisibility {
    Policy,
    Show,
    HideContent,
    Hide,
}

impl From<LockScreenVisibility> for StoredLockVisibility {
    fn from(visibility: LockScreenVisibility) -> Self {
        match visibility {
            LockScreenVisibility::Policy => Self::Policy,
            LockScreenVisibility::Show => Self::Show,
            LockScreenVisibility::HideContent => Self::HideContent,
            LockScreenVisibility::Hide => Self::Hide,
        }
    }
}

impl From<StoredLockVisibility> for LockScreenVisibility {
    fn from(visibility: StoredLockVisibility) -> Self {
        match visibility {
            StoredLockVisibility::Policy => Self::Policy,
            StoredLockVisibility::Show => Self::Show,
            StoredLockVisibility::HideContent => Self::HideContent,
            StoredLockVisibility::Hide => Self::Hide,
        }
    }
}
