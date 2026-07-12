//! Session-bus notification service shared by legacy and portal entry points.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use async_channel::{Receiver, Sender};
use rmac_notifications::{
    ActionInvocation, AppId, CloseReason, Closed, DeliveryPolicy, Notification, NotificationId,
    PostOutcome, Server, ServerError, Source, Time, TimeoutPolicy,
};
use zbus::connection::Builder;
use zbus::fdo;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{serialized::Context, serialized::Data, Endian, OwnedValue, Str, Value};
use zbus::{interface, Connection, Proxy};

const LEGACY_PATH: &str = "/org/freedesktop/Notifications";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const EVENT_CAPACITY: usize = 128;

#[derive(Clone)]
pub struct HistoryAuthority {
    center: Arc<Mutex<rmac_notifications_store::Center>>,
    store: rmac_notifications_store::Store,
    focus_connection: Option<Connection>,
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
        })
    }

    #[cfg(test)]
    fn empty_at(path: std::path::PathBuf) -> Self {
        Self {
            center: Arc::new(Mutex::new(rmac_notifications_store::Center::default())),
            store: rmac_notifications_store::Store::at(path),
            focus_connection: None,
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

    pub fn record(&self, event: &RuntimeEvent) -> Result<(), ServiceError> {
        let changed = {
            let mut center = self
                .center
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match event {
                RuntimeEvent::Posted {
                    outcome,
                    notification,
                } if outcome.delivery.history => {
                    if let Some(notification) = notification {
                        center.upsert(notification.as_ref().clone());
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
            let snapshot = self
                .center
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            self.store
                .save(&snapshot)
                .map_err(|_| ServiceError::History)?;
        }
        Ok(())
    }

    pub fn indicator(&self) -> rmac_notifications::Indicator {
        self.center
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .indicator()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceError {
    History,
    Bus,
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
    ) -> fdo::Result<u32> {
        let sender = authenticated_sender(&header)?;
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
        publish(
            &self.events,
            RuntimeEvent::Posted {
                outcome,
                notification,
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
        let request = super::portal(app_id, id, notification).map_err(invalid_wire)?;
        let policy = self.history.policy(request.source.app_id()).await;
        let (outcome, notification) = self
            .core
            .post_event(request, policy)
            .map_err(domain_error)?;
        publish(
            &self.events,
            RuntimeEvent::Posted {
                outcome,
                notification,
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
                self.emit_legacy_closed(id, 2).await?;
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
    let core = SharedCore::new(500, TimeoutPolicy::default());
    let history = HistoryAuthority::load().await?;
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
    };
    let connection = Builder::session()
        .map_err(|_| ServiceError::Bus)?
        .name("org.freedesktop.Notifications")
        .map_err(|_| ServiceError::Bus)?
        .name("org.freedesktop.impl.portal.desktop.rmac")
        .map_err(|_| ServiceError::Bus)?
        .serve_at(LEGACY_PATH, legacy)
        .map_err(|_| ServiceError::Bus)?
        .serve_at(PORTAL_PATH, portal)
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
    use rmac_notifications::protocol::{self, PortalInput};
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
        history
            .record(&RuntimeEvent::Posted {
                outcome: posted,
                notification: active.first().cloned().map(Box::new),
            })
            .unwrap();
        assert_eq!(
            rmac_notifications_store::Store::at(path.clone())
                .load()
                .unwrap()
                .center
                .history()
                .len(),
            1
        );

        history
            .record(&RuntimeEvent::Closed(Closed {
                id: posted.id,
                reason: CloseReason::Expired,
            }))
            .unwrap();
        assert_eq!(history.indicator().unread_count, 1);

        history
            .record(&RuntimeEvent::Closed(Closed {
                id: posted.id,
                reason: CloseReason::Dismissed,
            }))
            .unwrap();
        assert_eq!(history.indicator().unread_count, 0);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn public_protocol_metadata_is_truthful() {
        assert_eq!(protocol_categories().len(), 14);
        assert_eq!(protocol_button_purposes().len(), 7);
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
            core,
            events,
            history,
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
