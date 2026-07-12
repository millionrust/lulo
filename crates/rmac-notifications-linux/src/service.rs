//! Session-bus notification service shared by legacy and portal entry points.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use async_channel::{Receiver, Sender};
use rmac_notifications::{
    AppId, Closed, DeliveryPolicy, Notification, NotificationId, PostOutcome, Server, ServerError,
    Time, TimeoutPolicy,
};
use zbus::connection::Builder;
use zbus::fdo;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{interface, Connection};

const LEGACY_PATH: &str = "/org/freedesktop/Notifications";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const EVENT_CAPACITY: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeEvent {
    Posted(PostOutcome),
    Closed(Closed),
}

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
        let outcome = self
            .core
            .post(request, DeliveryPolicy::default())
            .map_err(domain_error)?;
        publish(&self.events, RuntimeEvent::Posted(outcome)).await?;
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
        let outcome = self
            .core
            .post(request, DeliveryPolicy::default())
            .map_err(domain_error)?;
        publish(&self.events, RuntimeEvent::Posted(outcome)).await
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

pub async fn serve() -> zbus::Result<(Connection, SharedCore, Receiver<RuntimeEvent>)> {
    let core = SharedCore::new(500, TimeoutPolicy::default());
    let (events, receiver) = async_channel::bounded(EVENT_CAPACITY);
    let legacy = LegacyInterface {
        core: core.clone(),
        events: events.clone(),
    };
    let portal = PortalInterface {
        core: core.clone(),
        events,
    };
    let connection = Builder::session()?
        .name("org.freedesktop.Notifications")?
        .name("org.freedesktop.impl.portal.desktop.rmac")?
        .serve_at(LEGACY_PATH, legacy)?
        .serve_at(PORTAL_PATH, portal)?
        .build()
        .await?;
    Ok((connection, core, receiver))
}

async fn publish(events: &Sender<RuntimeEvent>, event: RuntimeEvent) -> fdo::Result<()> {
    events
        .send(event)
        .await
        .map_err(|_| fdo::Error::Failed("notification runtime is unavailable".into()))
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
    use zbus::object_server::Interface;

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
        let legacy = LegacyInterface {
            core: core.clone(),
            events: events.clone(),
        };
        let portal = PortalInterface { core, events };
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
}
