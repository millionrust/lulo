//! Session-bus notification service shared by legacy and portal entry points.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use async_channel::{Receiver, Sender};
use rmac_notifications::{
    Action, ActionInvocation, AppId, CloseReason, Closed, DeliveryPolicy, Notification,
    NotificationId, PostOutcome, Server, ServerError, Source, Time, TimeoutPolicy,
};
use zbus::connection::Builder;
use zbus::fdo;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{serialized::Context, serialized::Data, Endian, OwnedValue, Str, Value};
use zbus::{interface, Connection, Proxy};

const LEGACY_PATH: &str = "/org/freedesktop/Notifications";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
pub const CENTER_BUS_NAME: &str = "org.rmac.NotificationCenter1";
pub const CENTER_PATH: &str = "/org/rmac/NotificationCenter1";
// Each event can transiently own one validated 4 MiB icon and 2 MiB sound.
// Backpressure therefore caps worst-case queued media at 192 MiB.
const EVENT_CAPACITY: usize = 32;
const MEDIA_ADMISSIONS: usize = 4;

#[derive(Clone)]
pub struct HistoryAuthority {
    center: Arc<Mutex<rmac_notifications_store::Center>>,
    store: rmac_notifications_store::Store,
    focus_connection: Option<Connection>,
    origins: crate::origin::Origins,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordOutcome {
    pub indicator: rmac_notifications::Indicator,
    pub changed: bool,
    pub persisted: bool,
}

impl std::fmt::Debug for HistoryAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("HistoryAuthority(<redacted>)")
    }
}

impl HistoryAuthority {
    async fn load() -> Result<Self, ServiceError> {
        let store = rmac_notifications_store::Store::from_environment()
            .map_err(|_| ServiceError::History)?;
        let center = store.load().map_err(|_| ServiceError::History)?.center;
        let focus_connection = Connection::session().await.map_err(|_| ServiceError::Bus)?;
        Ok(Self {
            center: Arc::new(Mutex::new(center)),
            store,
            focus_connection: Some(focus_connection),
            origins: crate::origin::Origins::default(),
        })
    }

    #[cfg(test)]
    fn empty_at(path: std::path::PathBuf) -> Self {
        Self {
            center: Arc::new(Mutex::new(rmac_notifications_store::Center::default())),
            store: rmac_notifications_store::Store::at(path),
            focus_connection: None,
            origins: crate::origin::Origins::default(),
        }
    }

    async fn policy(&self, app_id: &AppId) -> DeliveryPolicy {
        let base = self
            .center
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .policy(app_id)
            .delivery(false);
        let resolved = match &self.focus_connection {
            Some(connection) => {
                rmac_focus_linux::client::enforce_with_connection(connection, app_id, base).await
            }
            None => Err(rmac_focus_linux::client::Error::Connect),
        };
        resolved.unwrap_or(DeliveryPolicy {
            // Fail closed for banners while Focus state is unavailable;
            // policy-allowed history remains recoverable in the Center.
            focus_active: true,
            ..base
        })
    }

    pub fn record(&self, event: &RuntimeEvent) -> Option<RecordOutcome> {
        let changed = {
            let mut center = self
                .center
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match event {
                RuntimeEvent::Posted {
                    outcome,
                    notification,
                    ..
                } if outcome.delivery.history => {
                    if let Some(notification) = notification {
                        center.upsert(notification.as_ref().clone());
                        self.origins
                            .posted(notification.id, crate::origin::unix_ms_now());
                        true
                    } else {
                        false
                    }
                }
                RuntimeEvent::Closed(closed) if !matches!(closed.reason, CloseReason::Expired) => {
                    center.remove(closed.id)
                }
                RuntimeEvent::Posted { .. }
                | RuntimeEvent::Closed(_)
                | RuntimeEvent::ActionInvoked(_) => false,
            }
        };
        if changed {
            Some(self.finish(true))
        } else {
            None
        }
    }

    fn applications(&self) -> Vec<(String, crate::center::WireAppPolicy)> {
        self.center
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .applications()
            .into_iter()
            .map(|(app_id, policy)| {
                (
                    app_id.as_str().to_owned(),
                    crate::center::encode_policy(policy),
                )
            })
            .collect()
    }

    fn snapshot(&self, active: &[Notification]) -> crate::center::WireSnapshot {
        let center = self
            .center
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let active: HashMap<_, _> = active
            .iter()
            .map(|notification| (notification.id, notification))
            .collect();
        let history = center
            .groups()
            .into_iter()
            .flat_map(|group| {
                let active = &active;
                let app_id = group.app_id.as_str().to_owned();
                group.records.into_iter().map(move |record| {
                    let actionable = active
                        .get(&record.id)
                        .is_some_and(|candidate| same_action_record(candidate, record));
                    (
                        record.id.get(),
                        app_id.clone(),
                        record.content.title().to_owned(),
                        record.content.body().to_owned(),
                        crate::center::encode_priority(record.priority),
                        record.unread,
                        if actionable {
                            record
                                .default_action
                                .as_ref()
                                .and_then(visible_action_label)
                                .unwrap_or_default()
                        } else {
                            String::new()
                        },
                        if actionable {
                            record
                                .actions
                                .iter()
                                .enumerate()
                                .filter_map(|(index, action)| {
                                    Some((u8::try_from(index).ok()?, visible_action_label(action)?))
                                })
                                .collect()
                        } else {
                            Vec::new()
                        },
                    )
                })
            })
            .collect();
        let applications = center
            .applications()
            .into_iter()
            .map(|(app_id, policy)| {
                (
                    app_id.as_str().to_owned(),
                    crate::center::encode_policy(policy),
                )
            })
            .collect();
        (history, applications)
    }

    fn mark_read(&self, app_id: Option<&AppId>) -> RecordOutcome {
        let changed = self
            .center
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .mark_all_read(app_id);
        self.finish(changed)
    }

    fn clear(&self, app_id: Option<&AppId>) -> RecordOutcome {
        let changed = self
            .center
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear(app_id);
        self.finish(changed)
    }

    fn remove(&self, id: NotificationId) -> RecordOutcome {
        let changed = self
            .center
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id);
        self.finish(changed)
    }

    fn set_policy(
        &self,
        app_id: AppId,
        policy: rmac_notifications_store::AppPolicy,
    ) -> Result<RecordOutcome, ServiceError> {
        let changed = self
            .center
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_policy(app_id, policy)
            .map_err(|_| ServiceError::Invalid)?;
        Ok(self.finish(changed))
    }

    fn finish(&self, changed: bool) -> RecordOutcome {
        let snapshot = self
            .center
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let live: Vec<_> = snapshot
            .history()
            .iter()
            .map(|notification| notification.id)
            .collect();
        self.origins.retain(&live, crate::origin::unix_ms_now());
        RecordOutcome {
            indicator: snapshot.indicator(),
            changed,
            persisted: !changed || self.store.save(&snapshot).is_ok(),
        }
    }

    /// Display-only arrival time and sending application of each record.
    fn origins(&self) -> Vec<crate::origin::WireOrigin> {
        self.origins.wire(&self.ids())
    }

    /// Labels a legacy notification with the process that sent it.
    fn annotate_sender(
        &self,
        id: NotificationId,
        (desktop_id, executable): (Option<String>, Option<String>),
    ) {
        self.origins
            .sender(id, desktop_id, executable, crate::origin::unix_ms_now());
    }

    pub fn indicator(&self) -> rmac_notifications::Indicator {
        self.center
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .indicator()
    }

    fn ids(&self) -> Vec<NotificationId> {
        self.center
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .history()
            .iter()
            .map(|notification| notification.id)
            .collect()
    }

    fn action_is_visible(&self, active: &Notification, selection: &ActionSelection) -> bool {
        let center = self
            .center
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(record) = center
            .history()
            .iter()
            .find(|record| same_action_record(active, record))
        else {
            return false;
        };
        match selection {
            ActionSelection::Default => record
                .default_action
                .as_ref()
                .and_then(visible_action_label)
                .is_some(),
            ActionSelection::Button(index) => record
                .actions
                .get(*index)
                .and_then(visible_action_label)
                .is_some(),
            ActionSelection::Named(_) => false,
        }
    }
}

fn same_action_record(active: &Notification, record: &Notification) -> bool {
    active.id == record.id
        && active.source == record.source
        && active.updated_at == record.updated_at
        && active.default_action == record.default_action
        && active.actions == record.actions
}

fn visible_action_label(action: &Action) -> Option<String> {
    rmac_notifications::protocol::visible_action_label(action).map(Into::into)
}

#[derive(Clone, Debug)]
struct CenterInterface {
    history: HistoryAuthority,
    core: SharedCore,
    events: Sender<RuntimeEvent>,
}

#[interface(name = "org.rmac.NotificationCenter1")]
impl CenterInterface {
    fn state(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<(u32, bool)> {
        authenticated_sender(&header)?;
        let indicator = self.history.indicator();
        Ok((indicator.unread_count, indicator.has_urgent))
    }

    fn applications(
        &self,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<Vec<(String, crate::center::WireAppPolicy)>> {
        authenticated_sender(&header)?;
        Ok(self.history.applications())
    }

    fn origins(
        &self,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<Vec<crate::origin::WireOrigin>> {
        authenticated_sender(&header)?;
        Ok(self.history.origins())
    }

    async fn snapshot(
        &self,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<crate::center::WireSnapshot> {
        authenticated_sender(&header)?;
        let history = self.history.clone();
        let active = self.core.snapshot();
        Ok(blocking::unblock(move || history.snapshot(&active)).await)
    }

    async fn invoke(
        &self,
        id: u32,
        selection: u8,
        index: u8,
        activation_token: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<()> {
        authenticated_sender(&header)?;
        let id = NotificationId::from_protocol(id).ok_or_else(center_action_unavailable)?;
        let selection = match (selection, index) {
            (0, 0) => ActionSelection::Default,
            (1, index @ 0..=7) => ActionSelection::Button(index.into()),
            _ => return Err(center_invalid()),
        };
        let activation_token = validate_activation_token(activation_token)?;
        ServiceHandle {
            connection: connection.clone(),
            core: self.core.clone(),
            events: self.events.clone(),
            history: self.history.clone(),
        }
        .invoke_from_center(id, selection, activation_token)
        .await
        .map(drop)
        .map_err(center_action_error)
    }

    async fn mark_read(
        &self,
        app_id: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<(u32, bool)> {
        authenticated_sender(&header)?;
        let app_id = optional_app_id(app_id)?;
        let history = self.history.clone();
        let outcome = blocking::unblock(move || history.mark_read(app_id.as_ref())).await;
        complete_center_mutation(&emitter, outcome, false).await
    }

    async fn clear(
        &self,
        app_id: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<(u32, bool)> {
        authenticated_sender(&header)?;
        let app_id = optional_app_id(app_id)?;
        let history = self.history.clone();
        let outcome = blocking::unblock(move || history.clear(app_id.as_ref())).await;
        complete_center_mutation(&emitter, outcome, false).await
    }

    async fn remove(
        &self,
        id: u32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<(u32, bool)> {
        authenticated_sender(&header)?;
        let id = NotificationId::from_protocol(id).ok_or_else(center_invalid)?;
        let history = self.history.clone();
        let outcome = blocking::unblock(move || history.remove(id)).await;
        complete_center_mutation(&emitter, outcome, false).await
    }

    async fn set_policy(
        &self,
        app_id: &str,
        policy: crate::center::WireAppPolicy,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<(u32, bool)> {
        authenticated_sender(&header)?;
        let app_id = AppId::parse(app_id).map_err(|_| center_invalid())?;
        let policy = crate::center::decode_policy(policy).map_err(|_| center_invalid())?;
        let history = self.history.clone();
        let outcome = blocking::unblock(move || history.set_policy(app_id, policy))
            .await
            .map_err(|_| center_invalid())?;
        complete_center_mutation(&emitter, outcome, true).await
    }

    #[zbus(signal)]
    async fn changed(
        emitter: &SignalEmitter<'_>,
        unread_count: u32,
        has_urgent: bool,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn policies_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

fn optional_app_id(value: &str) -> fdo::Result<Option<AppId>> {
    if value.is_empty() {
        Ok(None)
    } else {
        AppId::parse(value).map(Some).map_err(|_| center_invalid())
    }
}

async fn complete_center_mutation(
    emitter: &SignalEmitter<'_>,
    outcome: RecordOutcome,
    policy_changed: bool,
) -> fdo::Result<(u32, bool)> {
    if outcome.changed {
        CenterInterface::changed(
            emitter,
            outcome.indicator.unread_count,
            outcome.indicator.has_urgent,
        )
        .await
        .map_err(fdo::Error::ZBus)?;
        if policy_changed {
            CenterInterface::policies_changed(emitter)
                .await
                .map_err(fdo::Error::ZBus)?;
        }
    }
    if !outcome.persisted {
        return Err(fdo::Error::Failed(
            "Notification Center changed but could not be saved".into(),
        ));
    }
    Ok((outcome.indicator.unread_count, outcome.indicator.has_urgent))
}

fn center_invalid() -> fdo::Error {
    fdo::Error::InvalidArgs("Notification Center request is invalid".into())
}

fn center_action_unavailable() -> fdo::Error {
    fdo::Error::InvalidArgs("Notification Center action is unavailable".into())
}

fn center_action_error(error: ActionError) -> fdo::Error {
    match error {
        ActionError::UnknownNotification | ActionError::UnknownAction => {
            center_action_unavailable()
        }
        ActionError::InvalidTarget | ActionError::InvalidApplication => {
            fdo::Error::Failed("Notification Center action is invalid".into())
        }
        ActionError::DocumentUnavailable => {
            fdo::Error::Failed("Notification document could not be opened".into())
        }
        ActionError::PersistentNotification
        | ActionError::Transport
        | ActionError::RuntimeUnavailable => {
            fdo::Error::Failed("Notification Center action could not be delivered".into())
        }
    }
}

fn validate_activation_token(value: &str) -> fdo::Result<Option<&str>> {
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > 4_096 || value.chars().any(char::is_control) {
        return Err(center_invalid());
    }
    Ok(Some(value))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceError {
    History,
    Bus,
    Invalid,
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "notification service failed ({self:?})")
    }
}

impl std::error::Error for ServiceError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeEvent {
    Posted {
        outcome: PostOutcome,
        notification: Option<Box<Notification>>,
        /// Validated presentation bytes live only in the bounded event stream.
        /// They are never copied into the reducer or durable Center history.
        media: crate::media::NotificationMedia,
    },
    Closed(Closed),
    ActionInvoked(ActionInvocation),
}

#[derive(Clone, Eq, PartialEq)]
pub enum ActionSelection {
    Default,
    Button(usize),
    Named(String),
}

impl std::fmt::Debug for ActionSelection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => formatter.write_str("Default"),
            Self::Button(index) => formatter.debug_tuple("Button").field(index).finish(),
            Self::Named(_) => formatter.write_str("Named(<redacted>)"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionError {
    UnknownNotification,
    UnknownAction,
    InvalidTarget,
    InvalidApplication,
    DocumentUnavailable,
    PersistentNotification,
    Transport,
    RuntimeUnavailable,
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "notification action failed ({self:?})")
    }
}

impl std::error::Error for ActionError {}

#[derive(Clone)]
pub struct SharedCore {
    inner: Arc<Mutex<Core>>,
}

impl std::fmt::Debug for SharedCore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SharedCore(<redacted>)")
    }
}

struct Core {
    server: Server,
    started: Instant,
}

impl SharedCore {
    pub fn new(history_limit: usize, timeout_policy: TimeoutPolicy) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Core {
                server: Server::new(history_limit, timeout_policy),
                started: Instant::now(),
            })),
        }
    }

    pub fn snapshot(&self) -> Vec<Notification> {
        self.lock().server.active().cloned().collect()
    }

    pub fn reserve_ids(&self, ids: impl IntoIterator<Item = NotificationId>) {
        self.lock().server.reserve_ids(ids);
    }

    pub fn post(
        &self,
        request: rmac_notifications::Request,
        policy: DeliveryPolicy,
    ) -> Result<PostOutcome, ServerError> {
        let mut core = self.lock();
        let now = monotonic_time(core.started);
        core.server.post(request, now, policy)
    }

    fn post_event(
        &self,
        request: rmac_notifications::Request,
        policy: DeliveryPolicy,
    ) -> Result<(PostOutcome, Option<Box<Notification>>), ServerError> {
        let mut core = self.lock();
        let now = monotonic_time(core.started);
        let outcome = core.server.post(request, now, policy)?;
        let notification = core
            .server
            .active()
            .find(|notification| notification.id == outcome.id)
            .cloned()
            .map(Box::new);
        Ok((outcome, notification))
    }

    pub fn withdraw(&self, app_id: &AppId, id: NotificationId) -> Result<Closed, ServerError> {
        self.lock().server.withdraw(app_id, id)
    }

    pub fn withdraw_portal(
        &self,
        app_id: &AppId,
        external_id: &str,
    ) -> Result<Closed, ServerError> {
        self.lock().server.withdraw_portal(app_id, external_id)
    }

    pub fn dismiss(&self, id: NotificationId) -> Result<Closed, ServerError> {
        self.lock().server.dismiss(id)
    }

    pub fn expire(&self) -> Vec<Closed> {
        let mut core = self.lock();
        let now = monotonic_time(core.started);
        core.server.expire(now)
    }

    pub fn expire_one(&self, id: NotificationId) -> Result<Closed, ServerError> {
        self.lock().server.expire_one(id)
    }

    pub fn invoke(
        &self,
        id: NotificationId,
        selection: &ActionSelection,
    ) -> Result<(ActionInvocation, Option<Closed>), ServerError> {
        let mut core = self.lock();
        match selection {
            ActionSelection::Default => core.server.invoke_default(id),
            ActionSelection::Button(index) => core.server.invoke_button(id, *index),
            ActionSelection::Named(action) => core.server.invoke(id, action),
        }
    }

    fn invoke_from_center(
        &self,
        history: &HistoryAuthority,
        id: NotificationId,
        selection: &ActionSelection,
    ) -> Result<(Source, ActionInvocation, Option<Closed>), ServerError> {
        let mut core = self.lock();
        let active = core
            .server
            .active()
            .find(|notification| notification.id == id)
            .cloned()
            .ok_or(ServerError::UnknownNotification)?;
        if !history.action_is_visible(&active, selection) {
            return Err(ServerError::UnknownAction);
        }
        let result = match selection {
            ActionSelection::Default => core.server.invoke_default(id),
            ActionSelection::Button(index) => core.server.invoke_button(id, *index),
            ActionSelection::Named(_) => Err(ServerError::UnknownAction),
        }?;
        Ok((active.source, result.0, result.1))
    }

    fn lock(&self) -> MutexGuard<'_, Core> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn monotonic_time(started: Instant) -> Time {
    Time(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX))
}

#[derive(Clone, Debug)]
pub struct LegacyInterface {
    core: SharedCore,
    events: Sender<RuntimeEvent>,
    history: HistoryAuthority,
}

#[interface(name = "org.freedesktop.Notifications")]
impl LegacyInterface {
    async fn get_capabilities(&self) -> Vec<String> {
        super::FREEDESKTOP_CAPABILITIES
            .iter()
            .map(|capability| (*capability).to_owned())
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        _app_name: String,
        replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<u32> {
        let sender = authenticated_sender(&header)?;
        let origin = crate::origin::sender_origin(connection, &sender).await;
        let request = super::freedesktop(
            sender,
            replaces_id,
            summary,
            body,
            actions,
            hints,
            expire_timeout,
        )
        .map_err(invalid_wire)?;
        let policy = self.history.policy(request.source.app_id()).await;
        let (outcome, notification) = self
            .core
            .post_event(request, policy)
            .map_err(domain_error)?;
        self.history.annotate_sender(outcome.id, origin);
        publish(
            &self.events,
            RuntimeEvent::Posted {
                outcome,
                notification,
                media: crate::media::NotificationMedia::default(),
            },
        )
        .await?;
        Ok(outcome.id.get())
    }

    async fn close_notification(
        &self,
        id: u32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let app_id = AppId::parse(authenticated_sender(&header)?).map_err(|_| {
            fdo::Error::AccessDenied("notification sender identity is invalid".into())
        })?;
        let id = NotificationId::from_protocol(id)
            .ok_or_else(|| fdo::Error::InvalidArgs("notification id must be nonzero".into()))?;
        let closed = self.core.withdraw(&app_id, id).map_err(domain_error)?;
        publish(&self.events, RuntimeEvent::Closed(closed.clone())).await?;
        emitter
            .notification_closed(closed.id.get(), 3)
            .await
            .map_err(fdo::Error::ZBus)
    }

    async fn get_server_information(&self) -> (String, String, String, String) {
        (
            "rmac Notifications".into(),
            "rmac".into(),
            env!("CARGO_PKG_VERSION").into(),
            "1.3".into(),
        )
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn activation_token(
        emitter: SignalEmitter<'_>,
        id: u32,
        activation_token: &str,
    ) -> zbus::Result<()>;
}

#[derive(Clone, Debug)]
pub struct PortalInterface {
    core: SharedCore,
    events: Sender<RuntimeEvent>,
    history: HistoryAuthority,
    media_decoder: MediaDecoder,
}

#[derive(Clone, Debug)]
struct MediaDecoder {
    permits: Sender<()>,
    available_permits: Receiver<()>,
    serial: Arc<Mutex<()>>,
}

impl MediaDecoder {
    fn new() -> Self {
        let (permits, available_permits) = async_channel::bounded(MEDIA_ADMISSIONS);
        for () in std::iter::repeat_n((), MEDIA_ADMISSIONS) {
            permits
                .try_send(())
                .expect("fresh media permit queue has exact capacity");
        }
        Self {
            permits,
            available_permits,
            serial: Arc::new(Mutex::new(())),
        }
    }

    fn try_acquire(&self) -> Option<MediaPermit> {
        self.available_permits.try_recv().ok()?;
        Some(MediaPermit {
            permits: self.permits.clone(),
        })
    }

    async fn decode(
        &self,
        app_id: String,
        id: String,
        notification: HashMap<String, OwnedValue>,
    ) -> Result<crate::PortalDecoded, crate::Error> {
        let serial = self.serial.clone();
        blocking::unblock(move || {
            let _guard = serial
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            super::portal_with_media(app_id, id, notification)
        })
        .await
    }
}

struct MediaPermit {
    permits: Sender<()>,
}

impl Drop for MediaPermit {
    fn drop(&mut self) {
        let _ = self.permits.try_send(());
    }
}

#[interface(name = "org.freedesktop.impl.portal.Notification")]
impl PortalInterface {
    async fn add_notification(
        &self,
        app_id: String,
        id: String,
        notification: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<()> {
        verify_portal_caller(connection, &header).await?;
        let _permit = self
            .media_decoder
            .try_acquire()
            .ok_or_else(|| fdo::Error::Failed("notification media decoder is busy".into()))?;
        let mut decoded = self
            .media_decoder
            .decode(app_id, id, notification)
            .await
            .map_err(invalid_wire)?;
        let policy = self.history.policy(decoded.request.source.app_id()).await;
        let (outcome, notification) = self
            .core
            .post_event(decoded.request, policy)
            .map_err(domain_error)?;
        decoded.media.retain_for(outcome.delivery);
        publish(
            &self.events,
            RuntimeEvent::Posted {
                outcome,
                notification,
                media: decoded.media,
            },
        )
        .await
    }

    async fn remove_notification(
        &self,
        app_id: String,
        id: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<()> {
        verify_portal_caller(connection, &header).await?;
        let app_id = AppId::parse(app_id)
            .map_err(|_| fdo::Error::InvalidArgs("invalid portal app id".into()))?;
        let closed = self
            .core
            .withdraw_portal(&app_id, &id)
            .map_err(domain_error)?;
        publish(&self.events, RuntimeEvent::Closed(closed)).await
    }

    #[zbus(property, name = "version")]
    async fn version(&self) -> u32 {
        2
    }

    #[zbus(property, name = "SupportedOptions")]
    async fn supported_options(&self) -> fdo::Result<HashMap<String, OwnedValue>> {
        Ok(HashMap::from([
            (
                "category".into(),
                string_array(protocol_categories()).map_err(fdo::Error::ZBus)?,
            ),
            (
                "button-purpose".into(),
                string_array(protocol_button_purposes()).map_err(fdo::Error::ZBus)?,
            ),
        ]))
    }

    #[zbus(signal)]
    async fn action_invoked(
        emitter: SignalEmitter<'_>,
        app_id: &str,
        id: &str,
        action: &str,
        parameter: Vec<OwnedValue>,
    ) -> zbus::Result<()>;
}

#[derive(Clone, Debug)]
pub struct ServiceHandle {
    connection: Connection,
    core: SharedCore,
    events: Sender<RuntimeEvent>,
    history: HistoryAuthority,
}

impl ServiceHandle {
    pub fn snapshot(&self) -> Vec<Notification> {
        self.core.snapshot()
    }

    pub fn history(&self) -> &HistoryAuthority {
        &self.history
    }

    pub async fn emit_indicator(
        &self,
        indicator: rmac_notifications::Indicator,
    ) -> Result<(), ServiceError> {
        let emitter =
            SignalEmitter::new(&self.connection, CENTER_PATH).map_err(|_| ServiceError::Bus)?;
        CenterInterface::changed(&emitter, indicator.unread_count, indicator.has_urgent)
            .await
            .map_err(|_| ServiceError::Bus)
    }

    pub async fn dismiss(&self, id: NotificationId) -> Result<Closed, ActionError> {
        let source = self.source(id)?;
        let closed = self.core.dismiss(id).map_err(action_domain_error)?;
        publish_action(&self.events, RuntimeEvent::Closed(closed.clone())).await?;
        if matches!(source, Source::Freedesktop { .. }) {
            self.emit_legacy_closed(id, 2).await?;
        }
        Ok(closed)
    }

    pub async fn expire_due(&self) -> Result<Vec<Closed>, ActionError> {
        let sources: HashMap<_, _> = self
            .core
            .snapshot()
            .into_iter()
            .map(|notification| (notification.id, notification.source))
            .collect();
        let closed = self.core.expire();
        for event in &closed {
            publish_action(&self.events, RuntimeEvent::Closed(event.clone())).await?;
            if matches!(sources.get(&event.id), Some(Source::Freedesktop { .. })) {
                self.emit_legacy_closed(event.id, 1).await?;
            }
        }
        Ok(closed)
    }

    pub async fn expire_banner(&self, id: NotificationId) -> Result<Closed, ActionError> {
        let source = self.source(id)?;
        let closed = self.core.expire_one(id).map_err(action_domain_error)?;
        publish_action(&self.events, RuntimeEvent::Closed(closed.clone())).await?;
        if matches!(source, Source::Freedesktop { .. }) {
            self.emit_legacy_closed(id, 1).await?;
        }
        Ok(closed)
    }

    pub async fn invoke(
        &self,
        id: NotificationId,
        selection: ActionSelection,
        activation_token: Option<&str>,
    ) -> Result<ActionInvocation, ActionError> {
        let source = self.source(id)?;
        let (invocation, closed) = self
            .core
            .invoke(id, &selection)
            .map_err(action_domain_error)?;
        self.finish_invocation(source, invocation, closed, activation_token)
            .await
    }

    async fn invoke_from_center(
        &self,
        id: NotificationId,
        selection: ActionSelection,
        activation_token: Option<&str>,
    ) -> Result<ActionInvocation, ActionError> {
        let (source, invocation, closed) = self
            .core
            .invoke_from_center(&self.history, id, &selection)
            .map_err(action_domain_error)?;
        self.finish_invocation(source, invocation, closed, activation_token)
            .await
    }

    async fn finish_invocation(
        &self,
        source: Source,
        invocation: ActionInvocation,
        closed: Option<Closed>,
        activation_token: Option<&str>,
    ) -> Result<ActionInvocation, ActionError> {
        self.dispatch_invocation(&source, &invocation, activation_token)
            .await?;
        publish_action(
            &self.events,
            RuntimeEvent::ActionInvoked(invocation.clone()),
        )
        .await?;
        if let Some(closed) = closed {
            publish_action(&self.events, RuntimeEvent::Closed(closed)).await?;
            if matches!(source, Source::Freedesktop { .. }) {
                self.emit_legacy_closed(invocation.notification_id, 2)
                    .await?;
            }
        }
        Ok(invocation)
    }

    fn source(&self, id: NotificationId) -> Result<Source, ActionError> {
        self.core
            .snapshot()
            .into_iter()
            .find(|notification| notification.id == id)
            .map(|notification| notification.source)
            .ok_or(ActionError::UnknownNotification)
    }

    async fn dispatch_invocation(
        &self,
        source: &Source,
        invocation: &ActionInvocation,
        activation_token: Option<&str>,
    ) -> Result<(), ActionError> {
        if let Some(path) = notification_document_path(source, invocation)? {
            return rmac_app_launch::open_document(path)
                .await
                .map_err(|_| ActionError::DocumentUnavailable);
        }
        match source {
            Source::Freedesktop { .. } => {
                let emitter = SignalEmitter::new(&self.connection, LEGACY_PATH)
                    .map_err(|_| ActionError::Transport)?;
                if let Some(token) = activation_token {
                    emitter
                        .activation_token(invocation.notification_id.get(), token)
                        .await
                        .map_err(|_| ActionError::Transport)?;
                }
                LegacyInterfaceSignals::action_invoked(
                    &emitter,
                    invocation.notification_id.get(),
                    &invocation.action_id,
                )
                .await
                .map_err(|_| ActionError::Transport)
            }
            Source::Portal {
                app_id,
                external_id: _,
            } if invocation.action_id.starts_with("app.") => {
                self.activate_application(app_id, invocation, activation_token)
                    .await
            }
            Source::Portal {
                app_id,
                external_id,
            } => {
                let emitter = SignalEmitter::new(&self.connection, PORTAL_PATH)
                    .map_err(|_| ActionError::Transport)?;
                PortalInterfaceSignals::action_invoked(
                    &emitter,
                    app_id.as_str(),
                    external_id,
                    &invocation.action_id,
                    portal_parameters(invocation.target.as_ref(), activation_token)?,
                )
                .await
                .map_err(|_| ActionError::Transport)
            }
        }
    }

    async fn activate_application(
        &self,
        app_id: &AppId,
        invocation: &ActionInvocation,
        activation_token: Option<&str>,
    ) -> Result<(), ActionError> {
        let object_path = application_object_path(app_id.as_str())?;
        let proxy = Proxy::new(
            &self.connection,
            app_id.as_str(),
            object_path.as_str(),
            "org.freedesktop.Application",
        )
        .await
        .map_err(|_| ActionError::InvalidApplication)?;
        let parameters = invocation
            .target
            .as_ref()
            .map(decode_target)
            .transpose()?
            .into_iter()
            .collect::<Vec<_>>();
        let platform_data = platform_data(activation_token);
        let action_name = invocation
            .action_id
            .strip_prefix("app.")
            .ok_or(ActionError::InvalidApplication)?;
        let _: () = proxy
            .call("ActivateAction", &(action_name, parameters, platform_data))
            .await
            .map_err(|_| ActionError::Transport)?;
        Ok(())
    }

    async fn emit_legacy_closed(&self, id: NotificationId, reason: u32) -> Result<(), ActionError> {
        SignalEmitter::new(&self.connection, LEGACY_PATH)
            .map_err(|_| ActionError::Transport)?
            .notification_closed(id.get(), reason)
            .await
            .map_err(|_| ActionError::Transport)
    }
}

pub async fn serve() -> Result<(ServiceHandle, Receiver<RuntimeEvent>), ServiceError> {
    let history = HistoryAuthority::load().await?;
    let core = SharedCore::new(500, TimeoutPolicy::default());
    core.reserve_ids(history.ids());
    let (events, receiver) = async_channel::bounded(EVENT_CAPACITY);
    let legacy = LegacyInterface {
        core: core.clone(),
        events: events.clone(),
        history: history.clone(),
    };
    let portal = PortalInterface {
        core: core.clone(),
        events: events.clone(),
        history: history.clone(),
        media_decoder: MediaDecoder::new(),
    };
    let center = CenterInterface {
        history: history.clone(),
        core: core.clone(),
        events: events.clone(),
    };
    let connection = Builder::session()
        .map_err(|_| ServiceError::Bus)?
        .name("org.freedesktop.Notifications")
        .map_err(|_| ServiceError::Bus)?
        .name("org.freedesktop.impl.portal.desktop.rmac")
        .map_err(|_| ServiceError::Bus)?
        .name(CENTER_BUS_NAME)
        .map_err(|_| ServiceError::Bus)?
        .serve_at(LEGACY_PATH, legacy)
        .map_err(|_| ServiceError::Bus)?
        .serve_at(PORTAL_PATH, portal)
        .map_err(|_| ServiceError::Bus)?
        .serve_at(CENTER_PATH, center)
        .map_err(|_| ServiceError::Bus)?
        .build()
        .await
        .map_err(|_| ServiceError::Bus)?;
    Ok((
        ServiceHandle {
            connection,
            core,
            events,
            history,
        },
        receiver,
    ))
}

async fn publish(events: &Sender<RuntimeEvent>, event: RuntimeEvent) -> fdo::Result<()> {
    events
        .send(event)
        .await
        .map_err(|_| fdo::Error::Failed("notification runtime is unavailable".into()))
}

async fn publish_action(
    events: &Sender<RuntimeEvent>,
    event: RuntimeEvent,
) -> Result<(), ActionError> {
    events
        .send(event)
        .await
        .map_err(|_| ActionError::RuntimeUnavailable)
}

fn action_domain_error(error: ServerError) -> ActionError {
    match error {
        ServerError::UnknownNotification => ActionError::UnknownNotification,
        ServerError::UnknownAction => ActionError::UnknownAction,
        ServerError::PersistentNotification => ActionError::PersistentNotification,
        ServerError::Invalid(_) | ServerError::WrongOwner | ServerError::ExhaustedIds => {
            ActionError::UnknownNotification
        }
    }
}

fn decode_target(target: &rmac_notifications::ActionTarget) -> Result<OwnedValue, ActionError> {
    if target.signature() != "v" {
        return Err(ActionError::InvalidTarget);
    }
    let data = Data::new(target.bytes(), Context::new_dbus(Endian::Little, 0));
    let (value, consumed): (OwnedValue, usize) =
        data.deserialize().map_err(|_| ActionError::InvalidTarget)?;
    if consumed != target.bytes().len() {
        return Err(ActionError::InvalidTarget);
    }
    Ok(value)
}

fn platform_data(activation_token: Option<&str>) -> HashMap<String, OwnedValue> {
    activation_token
        .map(|token| {
            HashMap::from([(
                "activation-token".into(),
                OwnedValue::from(Str::from(token.to_owned())),
            )])
        })
        .unwrap_or_default()
}

fn portal_parameters(
    target: Option<&rmac_notifications::ActionTarget>,
    activation_token: Option<&str>,
) -> Result<Vec<OwnedValue>, ActionError> {
    let mut parameters = Vec::with_capacity(2);
    if let Some(target) = target {
        parameters.push(decode_target(target)?);
    }
    parameters.push(OwnedValue::from(platform_data(activation_token)));
    Ok(parameters)
}

fn notification_document_path(
    source: &Source,
    invocation: &ActionInvocation,
) -> Result<Option<std::path::PathBuf>, ActionError> {
    if invocation.purpose.as_deref() != Some(rmac_notifications::protocol::DOCUMENT_OPEN_PURPOSE) {
        return Ok(None);
    }
    let Source::Portal { app_id, .. } = source else {
        return Err(ActionError::InvalidApplication);
    };
    if &invocation.app_id != app_id
        || !rmac_apps::identity::is_document_application(app_id.as_str())
    {
        return Err(ActionError::InvalidApplication);
    }
    let target = invocation
        .target
        .as_ref()
        .ok_or(ActionError::InvalidTarget)?;
    let uri = String::try_from(decode_target(target)?).map_err(|_| ActionError::InvalidTarget)?;
    let uri = url::Url::parse(&uri).map_err(|_| ActionError::InvalidTarget)?;
    if uri.scheme() != "file"
        || uri.host_str().is_some()
        || uri.query().is_some()
        || uri.fragment().is_some()
    {
        return Err(ActionError::InvalidTarget);
    }
    let path = uri
        .to_file_path()
        .map_err(|()| ActionError::InvalidTarget)?;
    if !path.is_absolute() {
        return Err(ActionError::InvalidTarget);
    }
    Ok(Some(path))
}

fn application_object_path(app_id: &str) -> Result<String, ActionError> {
    if app_id.is_empty() {
        return Err(ActionError::InvalidApplication);
    }
    let mut path = String::with_capacity(app_id.len() + 1);
    path.push('/');
    for character in app_id.chars() {
        path.push(match character {
            '.' => '/',
            '-' => '_',
            character if character.is_ascii_alphanumeric() || character == '_' => character,
            _ => return Err(ActionError::InvalidApplication),
        });
    }
    Ok(path)
}

fn authenticated_sender(header: &Header<'_>) -> fdo::Result<String> {
    header
        .sender()
        .map(|sender| sender.as_str().to_owned())
        .ok_or_else(|| fdo::Error::AccessDenied("notification sender is unavailable".into()))
}

async fn verify_portal_caller(connection: &Connection, header: &Header<'_>) -> fdo::Result<()> {
    let sender = header
        .sender()
        .ok_or_else(|| fdo::Error::AccessDenied("portal sender is unavailable".into()))?;
    let proxy = fdo::DBusProxy::new(connection)
        .await
        .map_err(fdo::Error::ZBus)?;
    let name = "org.freedesktop.portal.Desktop"
        .try_into()
        .map_err(|_| fdo::Error::Failed("portal service name is invalid".into()))?;
    let owner = proxy.get_name_owner(name).await?;
    if owner.as_str() == sender.as_str() {
        Ok(())
    } else {
        Err(fdo::Error::AccessDenied(
            "portal backend calls require xdg-desktop-portal".into(),
        ))
    }
}

fn invalid_wire(error: super::Error) -> fdo::Error {
    fdo::Error::InvalidArgs(error.to_string())
}

fn domain_error(error: ServerError) -> fdo::Error {
    match error {
        ServerError::WrongOwner => {
            fdo::Error::AccessDenied("notification belongs to another sender".into())
        }
        ServerError::UnknownNotification => {
            fdo::Error::InvalidArgs("notification does not exist".into())
        }
        ServerError::ExhaustedIds => {
            fdo::Error::LimitsExceeded("notification id space exhausted".into())
        }
        ServerError::Invalid(_) | ServerError::UnknownAction => {
            fdo::Error::InvalidArgs("notification request is invalid".into())
        }
        ServerError::PersistentNotification => {
            fdo::Error::AccessDenied("persistent notification cannot be dismissed".into())
        }
    }
}

fn string_array(values: &[&str]) -> zbus::Result<OwnedValue> {
    OwnedValue::try_from(Value::new(
        values
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
    ))
    .map_err(Into::into)
}

fn protocol_categories() -> &'static [&'static str] {
    rmac_notifications::protocol::SUPPORTED_CATEGORIES
}

fn protocol_button_purposes() -> &'static [&'static str] {
    rmac_notifications::protocol::SUPPORTED_BUTTON_PURPOSES
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_notifications::protocol::{
        self, FreedesktopHints, FreedesktopInput, PortalButton, PortalInput,
    };
    use rmac_notifications::ActionTarget;
    use std::sync::atomic::{AtomicU64, Ordering};
    use zbus::object_server::Interface;
    use zbus::zvariant::to_bytes;

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn history_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!(
                "rmac-notification-service-{}-{label}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ))
            .join("history.json")
    }

    fn string_target(value: &str) -> ActionTarget {
        let value = OwnedValue::from(Str::from(value.to_owned()));
        let encoded = to_bytes(Context::new_dbus(Endian::Little, 0), &value).unwrap();
        ActionTarget::new("v", encoded.bytes().to_vec()).unwrap()
    }

    fn document_invocation(
        app_id: &str,
        uri: &str,
    ) -> (Source, rmac_notifications::ActionInvocation) {
        let app_id = AppId::parse(app_id).unwrap();
        (
            Source::portal(app_id.clone(), "document-ready").unwrap(),
            rmac_notifications::ActionInvocation {
                notification_id: NotificationId::from_protocol(1).unwrap(),
                app_id,
                action_id: "open-document".into(),
                target: Some(string_target(uri)),
                purpose: Some(rmac_notifications::protocol::DOCUMENT_OPEN_PURPOSE.into()),
            },
        )
    }

    #[test]
    fn shared_core_keeps_one_authority_for_both_protocols() {
        let core = SharedCore::new(10, TimeoutPolicy::default());
        let portal = protocol::portal(PortalInput {
            app_id: "org.example.App".into(),
            id: "one".into(),
            title: Some("First".into()),
            ..PortalInput::default()
        })
        .unwrap();
        let first = core.post(portal, DeliveryPolicy::default()).unwrap();
        let replacement = protocol::portal(PortalInput {
            app_id: "org.example.App".into(),
            id: "one".into(),
            title: Some("Updated".into()),
            ..PortalInput::default()
        })
        .unwrap();
        let replacement = core.post(replacement, DeliveryPolicy::default()).unwrap();
        assert_eq!(replacement.id, first.id);
        assert_eq!(core.snapshot().len(), 1);
        assert_eq!(core.snapshot()[0].content.title(), "Updated");
    }

    #[test]
    fn runtime_events_persist_history_but_expiry_does_not_delete_it() {
        let path = history_path("events");
        let history = HistoryAuthority::empty_at(path.clone());
        let core = SharedCore::new(10, TimeoutPolicy::default());
        let request = protocol::portal(PortalInput {
            app_id: "org.example.App".into(),
            id: "one".into(),
            title: Some("Private title".into()),
            ..PortalInput::default()
        })
        .unwrap();
        let posted = core.post(request, DeliveryPolicy::default()).unwrap();
        let active = core.snapshot();
        core.withdraw_portal(&AppId::parse("org.example.App").unwrap(), "one")
            .unwrap();
        assert!(history
            .record(&RuntimeEvent::Posted {
                outcome: posted,
                notification: active.first().cloned().map(Box::new),
                media: crate::media::NotificationMedia::default(),
            })
            .is_some());
        assert_eq!(
            rmac_notifications_store::Store::at(path.clone())
                .load()
                .unwrap()
                .center
                .history()
                .len(),
            1
        );

        assert!(history
            .record(&RuntimeEvent::Closed(Closed {
                id: posted.id,
                reason: CloseReason::Expired,
            }))
            .is_none());
        assert_eq!(history.indicator().unread_count, 1);

        assert!(history
            .record(&RuntimeEvent::Closed(Closed {
                id: posted.id,
                reason: CloseReason::Dismissed,
            }))
            .is_some());
        assert_eq!(history.indicator().unread_count, 0);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn save_failure_keeps_the_live_indicator_truthful() {
        let root = history_path("save-failure");
        std::fs::create_dir_all(root.parent().unwrap()).unwrap();
        let blocker = root.parent().unwrap().join("not-a-directory");
        std::fs::write(&blocker, b"block").unwrap();
        let history = HistoryAuthority::empty_at(blocker.join("history.json"));
        let core = SharedCore::new(10, TimeoutPolicy::default());
        let request = protocol::portal(PortalInput {
            app_id: "org.example.App".into(),
            id: "one".into(),
            title: Some("Private title".into()),
            ..PortalInput::default()
        })
        .unwrap();
        let (outcome, notification) = core.post_event(request, DeliveryPolicy::default()).unwrap();
        let recorded = history
            .record(&RuntimeEvent::Posted {
                outcome,
                notification,
                media: crate::media::NotificationMedia::default(),
            })
            .unwrap();
        assert_eq!(recorded.indicator.unread_count, 1);
        assert!(!recorded.persisted);
        assert_eq!(history.indicator().unread_count, 1);
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn center_mutations_refresh_from_one_persisted_authority() {
        let path = history_path("mutations");
        let history = HistoryAuthority::empty_at(path.clone());
        let core = SharedCore::new(10, TimeoutPolicy::default());
        let request = protocol::portal(PortalInput {
            app_id: "org.example.App".into(),
            id: "one".into(),
            title: Some("Private title".into()),
            ..PortalInput::default()
        })
        .unwrap();
        let (outcome, notification) = core.post_event(request, DeliveryPolicy::default()).unwrap();
        history
            .record(&RuntimeEvent::Posted {
                outcome,
                notification,
                media: crate::media::NotificationMedia::default(),
            })
            .unwrap();
        let applications = history.applications();
        assert_eq!(applications.len(), 1);
        assert_ne!(applications[0].0, "");
        let (records, snapshot_applications) = history.snapshot(&core.snapshot());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].0, outcome.id.get());
        assert_eq!(snapshot_applications, applications);

        let app_id = AppId::parse("org.example.App").unwrap();
        let read = history.mark_read(Some(&app_id));
        assert!(read.changed);
        assert!(read.persisted);
        assert_eq!(read.indicator.unread_count, 0);

        let policy = rmac_notifications_store::AppPolicy {
            banners: false,
            sounds: false,
            ..rmac_notifications_store::AppPolicy::default()
        };
        let changed = history.set_policy(app_id.clone(), policy).unwrap();
        assert!(changed.changed);
        assert!(changed.persisted);
        assert_eq!(
            history.applications()[0].1,
            crate::center::encode_policy(policy)
        );

        let cleared = history.clear(Some(&app_id));
        assert!(cleared.changed);
        assert!(cleared.persisted);
        let loaded = rmac_notifications_store::Store::at(path.clone())
            .load()
            .unwrap();
        assert_eq!(loaded.center.policy(&app_id), policy);
        assert_eq!(loaded.center.indicator().unread_count, 0);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn center_projects_actions_only_for_the_exact_live_record() {
        let path = history_path("live-actions");
        let history = HistoryAuthority::empty_at(path.clone());
        let core = SharedCore::new(10, TimeoutPolicy::default());
        let request = protocol::freedesktop(FreedesktopInput {
            authenticated_app_id: "org.example.Chat".into(),
            summary: "Private title".into(),
            actions: vec![
                "default".into(),
                "Open".into(),
                "archive".into(),
                "Archive".into(),
            ],
            hints: FreedesktopHints {
                resident: true,
                ..FreedesktopHints::default()
            },
            expire_timeout: 0,
            ..FreedesktopInput::default()
        })
        .unwrap();
        let (outcome, notification) = core.post_event(request, DeliveryPolicy::default()).unwrap();
        history
            .record(&RuntimeEvent::Posted {
                outcome,
                notification,
                media: crate::media::NotificationMedia::default(),
            })
            .unwrap();

        let (live, _) = history.snapshot(&core.snapshot());
        assert_eq!(live[0].6, "Open");
        assert_eq!(live[0].7, vec![(0, "Archive".to_owned())]);

        let mut same_time_replacement = core.snapshot();
        same_time_replacement[0].actions[0] = Action::new("archive", "Archive", None)
            .unwrap()
            .with_purpose("call.accept")
            .unwrap();
        let (mismatched, _) = history.snapshot(&same_time_replacement);
        assert!(mismatched[0].6.is_empty());
        assert!(mismatched[0].7.is_empty());

        let (_, default, default_closed) = core
            .invoke_from_center(&history, outcome.id, &ActionSelection::Default)
            .unwrap();
        assert_eq!(default.action_id, "default");
        assert_eq!(default_closed, None);
        let (_, archive, archive_closed) = core
            .invoke_from_center(&history, outcome.id, &ActionSelection::Button(0))
            .unwrap();
        assert_eq!(archive.action_id, "archive");
        assert_eq!(archive_closed, None);
        assert_eq!(
            core.invoke_from_center(
                &history,
                outcome.id,
                &ActionSelection::Named("archive".into())
            ),
            Err(ServerError::UnknownAction)
        );

        let (after_restart, _) = history.snapshot(&[]);
        assert!(after_restart[0].6.is_empty());
        assert!(after_restart[0].7.is_empty());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn center_keeps_original_positions_when_hidden_actions_are_filtered() {
        let path = history_path("action-position");
        let history = HistoryAuthority::empty_at(path.clone());
        let core = SharedCore::new(10, TimeoutPolicy::default());
        let request = protocol::portal(PortalInput {
            app_id: "org.example.Chat".into(),
            id: "message-1".into(),
            title: Some("Private title".into()),
            buttons: vec![
                PortalButton {
                    action: "app.custom".into(),
                    purpose: Some("system.custom-alert".into()),
                    ..PortalButton::default()
                },
                PortalButton {
                    action: "app.reply".into(),
                    purpose: Some("im.reply-with-text".into()),
                    ..PortalButton::default()
                },
            ],
            ..PortalInput::default()
        })
        .unwrap();
        let (outcome, notification) = core.post_event(request, DeliveryPolicy::default()).unwrap();
        history
            .record(&RuntimeEvent::Posted {
                outcome,
                notification,
                media: crate::media::NotificationMedia::default(),
            })
            .unwrap();

        let (records, _) = history.snapshot(&core.snapshot());
        assert_eq!(records[0].7, vec![(1, "Reply".to_owned())]);
        assert_eq!(
            core.invoke_from_center(&history, outcome.id, &ActionSelection::Button(0)),
            Err(ServerError::UnknownAction)
        );
        let (_, reply, _) = core
            .invoke_from_center(&history, outcome.id, &ActionSelection::Button(1))
            .unwrap();
        assert_eq!(reply.action_id, "app.reply");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn center_uses_only_explicit_or_standardized_action_labels() {
        let reply = Action::new("reply", "", None)
            .unwrap()
            .with_purpose("im.reply-with-text")
            .unwrap();
        let custom = Action::new("custom", "", None)
            .unwrap()
            .with_purpose("system.custom-alert")
            .unwrap();
        let explicit = Action::new("archive", "Archive", None).unwrap();
        assert_eq!(visible_action_label(&reply).as_deref(), Some("Reply"));
        assert_eq!(visible_action_label(&custom), None);
        assert_eq!(visible_action_label(&explicit).as_deref(), Some("Archive"));
    }

    #[test]
    fn center_activation_tokens_are_bounded_opaque_values() {
        assert_eq!(validate_activation_token("").unwrap(), None);
        assert_eq!(
            validate_activation_token("wayland:seat-7:opaque").unwrap(),
            Some("wayland:seat-7:opaque")
        );
        assert!(validate_activation_token("contains\ncontrol").is_err());
        assert!(validate_activation_token(&"x".repeat(4_097)).is_err());
    }

    #[test]
    fn public_protocol_metadata_is_truthful() {
        assert_eq!(protocol_categories().len(), 14);
        assert_eq!(protocol_button_purposes().len(), 8);
        assert!(protocol_button_purposes()
            .contains(&rmac_notifications::protocol::DOCUMENT_OPEN_PURPOSE));
        assert_eq!(NotificationId::from_protocol(0), None);
        assert_eq!(NotificationId::from_protocol(7).unwrap().get(), 7);
    }

    #[test]
    fn generated_interfaces_expose_the_standard_contracts() {
        let core = SharedCore::new(10, TimeoutPolicy::default());
        let (events, _receiver) = async_channel::bounded(4);
        let history = HistoryAuthority::empty_at(
            std::env::temp_dir().join("rmac-notification-interface-test.json"),
        );
        let legacy = LegacyInterface {
            core: core.clone(),
            events: events.clone(),
            history: history.clone(),
        };
        let portal = PortalInterface {
            core: core.clone(),
            events: events.clone(),
            history: history.clone(),
            media_decoder: MediaDecoder::new(),
        };
        let center = CenterInterface {
            history,
            core,
            events,
        };
        let mut legacy_xml = String::new();
        legacy.introspect_to_writer(&mut legacy_xml, 0);
        assert!(legacy_xml.contains("org.freedesktop.Notifications"));
        assert!(legacy_xml.contains("method name=\"Notify\""));
        assert!(legacy_xml.contains("method name=\"CloseNotification\""));
        assert!(legacy_xml.contains("signal name=\"NotificationClosed\""));
        assert!(legacy_xml.contains("signal name=\"ActionInvoked\""));

        let mut portal_xml = String::new();
        portal.introspect_to_writer(&mut portal_xml, 0);
        assert!(portal_xml.contains("org.freedesktop.impl.portal.Notification"));
        assert!(portal_xml.contains("method name=\"AddNotification\""));
        assert!(portal_xml.contains("method name=\"RemoveNotification\""));
        assert!(portal_xml.contains("property name=\"version\" type=\"u\""));
        assert!(portal_xml.contains("property name=\"SupportedOptions\" type=\"a{sv}\""));

        let mut center_xml = String::new();
        center.introspect_to_writer(&mut center_xml, 0);
        assert!(center_xml.contains("org.rmac.NotificationCenter1"));
        assert!(center_xml.contains("method name=\"State\""));
        assert!(center_xml.contains("method name=\"Applications\""));
        assert!(center_xml.contains("method name=\"Snapshot\""));
        assert!(center_xml.contains("method name=\"Invoke\""));
        assert!(center_xml.contains("method name=\"MarkRead\""));
        assert!(center_xml.contains("method name=\"Clear\""));
        assert!(center_xml.contains("method name=\"SetPolicy\""));
        assert!(center_xml.contains("signal name=\"Changed\""));
        assert!(center_xml.contains("signal name=\"PoliciesChanged\""));
    }

    #[test]
    fn media_decoder_admission_is_bounded_and_returned_on_drop() {
        let decoder = MediaDecoder::new();
        let mut permits = (0..MEDIA_ADMISSIONS)
            .map(|_| decoder.try_acquire().unwrap())
            .collect::<Vec<_>>();
        assert!(decoder.try_acquire().is_none());
        permits.pop();
        assert!(decoder.try_acquire().is_some());
    }

    #[test]
    fn opaque_targets_round_trip_and_platform_data_stays_last() {
        let original = OwnedValue::from(Str::from("conversation-8472".to_owned()));
        let encoded = to_bytes(Context::new_dbus(Endian::Little, 0), &original).unwrap();
        let target = ActionTarget::new("v", encoded.bytes().to_vec()).unwrap();
        let decoded = decode_target(&target).unwrap();
        assert_eq!(String::try_from(decoded).unwrap(), "conversation-8472");

        let parameters = portal_parameters(Some(&target), Some("activation-8472")).unwrap();
        assert_eq!(parameters.len(), 2);
        let platform =
            HashMap::<String, OwnedValue>::try_from(parameters.into_iter().last().unwrap())
                .unwrap();
        assert_eq!(
            String::try_from(platform.into_iter().next().unwrap().1).unwrap(),
            "activation-8472"
        );
    }

    #[test]
    fn first_party_document_action_decodes_one_local_file_uri() {
        let (source, invocation) = document_invocation(
            rmac_apps::identity::TEXT_EDITOR,
            "file:///home/private/Report%20Draft.txt",
        );
        assert_eq!(
            notification_document_path(&source, &invocation),
            Ok(Some(std::path::PathBuf::from(
                "/home/private/Report Draft.txt"
            )))
        );
        let debug = format!("{invocation:?}");
        assert!(!debug.contains("Report"));
        assert!(!debug.contains("open-document"));
    }

    #[test]
    fn document_action_rejects_untrusted_sources_and_ambiguous_targets() {
        let (untrusted_source, untrusted) =
            document_invocation("org.example.TextEditor", "file:///home/private/report.txt");
        assert_eq!(
            notification_document_path(&untrusted_source, &untrusted),
            Err(ActionError::InvalidApplication)
        );

        let (remote_source, remote) = document_invocation(
            rmac_apps::identity::FILES,
            "file://server/private/report.txt",
        );
        assert_eq!(
            notification_document_path(&remote_source, &remote),
            Err(ActionError::InvalidTarget)
        );

        let (query_source, query) = document_invocation(
            rmac_apps::identity::NOTES,
            "file:///home/private/report.txt?revision=secret",
        );
        assert_eq!(
            notification_document_path(&query_source, &query),
            Err(ActionError::InvalidTarget)
        );

        let (ordinary_source, mut ordinary) =
            document_invocation("org.example.App", "https://example.com/private");
        ordinary.purpose = None;
        assert_eq!(
            notification_document_path(&ordinary_source, &ordinary),
            Ok(None)
        );

        let (missing_source, mut missing) = document_invocation(
            rmac_apps::identity::TEXT_EDITOR,
            "file:///home/private/report.txt",
        );
        missing.target = None;
        assert_eq!(
            notification_document_path(&missing_source, &missing),
            Err(ActionError::InvalidTarget)
        );
    }

    #[test]
    fn application_object_path_follows_desktop_entry_activation_rules() {
        assert_eq!(
            application_object_path("org.example.Photo-Viewer").unwrap(),
            "/org/example/Photo_Viewer"
        );
        assert_eq!(
            application_object_path("org.example.invalid/path"),
            Err(ActionError::InvalidApplication)
        );
        assert!(
            !format!("{:?}", ActionSelection::Named("secret-action".into()))
                .contains("secret-action")
        );
    }
}
