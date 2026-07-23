//! Typed clients for the session-owned Focus authority.

use async_channel::Sender;
use futures_util::StreamExt as _;
use rmac_focus::{ActivationSource, Config};
use rmac_focus_runtime::Projection;
use rmac_notifications::{AppId, DeliveryPolicy};

use crate::service::{
    activation_source, decode_configuration, decode_policy, encode_configuration, encode_policy,
    projection, WireConfiguration, WirePolicy, WireSettings, WireState, SCHEDULED_DISABLE_DETAIL,
};

#[cfg(test)]
use crate::service::{BUS_NAME, INTERFACE_NAME, OBJECT_PATH};

#[zbus::proxy(
    interface = "org.rmac.Focus1",
    default_service = "org.rmac.Focus1",
    default_path = "/org/rmac/Focus1"
)]
trait Focus {
    fn state(&self) -> zbus::Result<WireState>;
    fn set_enabled(&self, enabled: bool) -> zbus::Result<WireState>;
    fn activate(&self, mode_id: &str, duration_ms: u64) -> zbus::Result<WireState>;
    fn disable(&self) -> zbus::Result<WireState>;
    fn delivery_policy(&self, app_id: &str, base: WirePolicy) -> zbus::Result<WirePolicy>;
    fn configuration(&self) -> zbus::Result<WireConfiguration>;
    fn settings(&self) -> zbus::Result<WireSettings>;
    fn replace_configuration(&self, configuration: WireConfiguration) -> zbus::Result<WireState>;

    #[zbus(signal)]
    fn changed(&self, state: WireState) -> zbus::Result<()>;

    #[zbus(signal)]
    fn configuration_changed(&self) -> zbus::Result<()>;
}

#[derive(Clone, Eq, PartialEq)]
pub struct Snapshot {
    pub projection: Projection,
    pub mode_id: Option<String>,
    pub source: Option<ActivationSource>,
    pub persistence_healthy: bool,
}

#[derive(Clone, Eq, PartialEq)]
pub struct SettingsSnapshot {
    pub configuration: Config,
    pub state: Snapshot,
}

impl std::fmt::Debug for SettingsSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SettingsSnapshot")
            .field("configuration", &self.configuration)
            .field("state", &self.state)
            .finish()
    }
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Snapshot")
            .field("projection", &self.projection)
            .field("mode_id", &self.mode_id.as_ref().map(|_| "<redacted>"))
            .field("source", &self.source)
            .field("persistence_healthy", &self.persistence_healthy)
            .finish()
    }
}

pub fn state() -> Result<Snapshot, Error> {
    let connection = zbus::blocking::Connection::session().map_err(|_| Error::Connect)?;
    let proxy = FocusProxyBlocking::new(&connection).map_err(|_| Error::Connect)?;
    decode(proxy.state().map_err(|_| Error::Call)?)
}

pub fn set_enabled(enabled: bool) -> Result<Snapshot, Error> {
    let connection = zbus::blocking::Connection::session().map_err(|_| Error::Connect)?;
    let proxy = FocusProxyBlocking::new(&connection).map_err(|_| Error::Connect)?;
    ensure_persisted(decode(proxy.set_enabled(enabled).map_err(call_error)?)?)
}

pub fn activate(mode_id: &str, duration_ms: u64) -> Result<Snapshot, Error> {
    let connection = zbus::blocking::Connection::session().map_err(|_| Error::Connect)?;
    let proxy = FocusProxyBlocking::new(&connection).map_err(|_| Error::Connect)?;
    ensure_persisted(decode(
        proxy.activate(mode_id, duration_ms).map_err(call_error)?,
    )?)
}

pub fn disable() -> Result<Snapshot, Error> {
    let connection = zbus::blocking::Connection::session().map_err(|_| Error::Connect)?;
    let proxy = FocusProxyBlocking::new(&connection).map_err(|_| Error::Connect)?;
    ensure_persisted(decode(proxy.disable().map_err(call_error)?)?)
}

pub fn configuration() -> Result<Config, Error> {
    let connection = zbus::blocking::Connection::session().map_err(|_| Error::Connect)?;
    let proxy = FocusProxyBlocking::new(&connection).map_err(|_| Error::Connect)?;
    decode_configuration(proxy.configuration().map_err(call_error)?).map_err(|_| Error::Protocol)
}

pub fn settings() -> Result<SettingsSnapshot, Error> {
    let connection = zbus::blocking::Connection::session().map_err(|_| Error::Connect)?;
    let proxy = FocusProxyBlocking::new(&connection).map_err(|_| Error::Connect)?;
    decode_settings(proxy.settings().map_err(call_error)?)
}

pub fn replace_configuration(configuration: &Config) -> Result<Snapshot, Error> {
    let connection = zbus::blocking::Connection::session().map_err(|_| Error::Connect)?;
    let proxy = FocusProxyBlocking::new(&connection).map_err(|_| Error::Connect)?;
    ensure_persisted(decode(
        proxy
            .replace_configuration(encode_configuration(configuration))
            .map_err(call_error)?,
    )?)
}

pub async fn enforce(app_id: &AppId, base: DeliveryPolicy) -> Result<DeliveryPolicy, Error> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|_| Error::Connect)?;
    enforce_with_connection(&connection, app_id, base).await
}

pub async fn enforce_with_connection(
    connection: &zbus::Connection,
    app_id: &AppId,
    base: DeliveryPolicy,
) -> Result<DeliveryPolicy, Error> {
    let proxy = FocusProxy::new(connection)
        .await
        .map_err(|_| Error::Connect)?;
    let wire = encode_policy(base);
    let result = proxy
        .delivery_policy(app_id.as_str(), wire)
        .await
        .map_err(call_error)?;
    decode_policy(result).map_err(|_| Error::Protocol)
}

pub async fn watch(sender: Sender<Result<Projection, String>>) -> Result<(), Error> {
    loop {
        match watch_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {
                if sender
                    .send(Err("Focus authority stopped; reconnecting".into()))
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
            Err(error) => {
                if sender.send(Err(error.to_string())).await.is_err() {
                    return Ok(());
                }
            }
        }
        let timer = futures_util::FutureExt::fuse(async_io::Timer::after(
            std::time::Duration::from_secs(1),
        ));
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(timer, closed);
        futures_util::select! {
            _ = timer => {},
            _ = closed => return Ok(()),
        }
    }
}

pub async fn watch_configuration(sender: Sender<Result<Config, String>>) -> Result<(), Error> {
    loop {
        match watch_configuration_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {
                if sender
                    .send(Err("Focus authority stopped; reconnecting".into()))
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
            Err(error) => {
                if sender.send(Err(error.to_string())).await.is_err() {
                    return Ok(());
                }
            }
        }
        let timer = futures_util::FutureExt::fuse(async_io::Timer::after(
            std::time::Duration::from_secs(1),
        ));
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(timer, closed);
        futures_util::select! {
            _ = timer => {},
            _ = closed => return Ok(()),
        }
    }
}

/// Publishes complete configuration and live-state pairs for Settings clients.
///
/// Both signal streams are installed before the initial reads, closing the
/// subscribe/read race. Any state or configuration signal triggers a complete
/// reread so consumers never have to merge independently versioned payloads.
pub async fn watch_settings(sender: Sender<Result<SettingsSnapshot, String>>) -> Result<(), Error> {
    loop {
        match watch_settings_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {
                if sender
                    .send(Err("Focus authority stopped; reconnecting".into()))
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
            Err(error) => {
                if sender.send(Err(error.to_string())).await.is_err() {
                    return Ok(());
                }
            }
        }
        let timer = futures_util::FutureExt::fuse(async_io::Timer::after(
            std::time::Duration::from_secs(1),
        ));
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(timer, closed);
        futures_util::select! {
            _ = timer => {},
            _ = closed => return Ok(()),
        }
    }
}

async fn watch_settings_once(
    sender: &Sender<Result<SettingsSnapshot, String>>,
) -> Result<(), Error> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|_| Error::Connect)?;
    let proxy = FocusProxy::new(&connection)
        .await
        .map_err(|_| Error::Connect)?;
    let state_changes = proxy
        .receive_changed()
        .await
        .map_err(|_| Error::Subscribe)?;
    let configuration_changes = proxy
        .receive_configuration_changed()
        .await
        .map_err(|_| Error::Subscribe)?;
    publish_settings(&proxy, sender).await?;
    futures_util::pin_mut!(state_changes, configuration_changes);
    loop {
        let state_change = futures_util::FutureExt::fuse(state_changes.next());
        let configuration_change = futures_util::FutureExt::fuse(configuration_changes.next());
        futures_util::pin_mut!(state_change, configuration_change);
        let open = futures_util::select! {
            change = state_change => change.is_some(),
            change = configuration_change => change.is_some(),
        };
        if !open {
            return Ok(());
        }
        publish_settings(&proxy, sender).await?;
    }
}

async fn publish_settings(
    proxy: &FocusProxy<'_>,
    sender: &Sender<Result<SettingsSnapshot, String>>,
) -> Result<(), Error> {
    let snapshot = decode_settings(proxy.settings().await.map_err(call_error)?)?;
    sender.send(Ok(snapshot)).await.map_err(|_| Error::Publish)
}

async fn watch_configuration_once(sender: &Sender<Result<Config, String>>) -> Result<(), Error> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|_| Error::Connect)?;
    let proxy = FocusProxy::new(&connection)
        .await
        .map_err(|_| Error::Connect)?;
    let mut changes = proxy
        .receive_configuration_changed()
        .await
        .map_err(|_| Error::Subscribe)?;
    let initial = decode_configuration(proxy.configuration().await.map_err(call_error)?)
        .map_err(|_| Error::Protocol)?;
    sender.send(Ok(initial)).await.map_err(|_| Error::Publish)?;
    while changes.next().await.is_some() {
        let configuration = decode_configuration(proxy.configuration().await.map_err(call_error)?)
            .map_err(|_| Error::Protocol)?;
        if sender.send(Ok(configuration)).await.is_err() {
            return Ok(());
        }
    }
    Ok(())
}

async fn watch_once(sender: &Sender<Result<Projection, String>>) -> Result<(), Error> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|_| Error::Connect)?;
    let proxy = FocusProxy::new(&connection)
        .await
        .map_err(|_| Error::Connect)?;
    let mut changes = proxy
        .receive_changed()
        .await
        .map_err(|_| Error::Subscribe)?;
    let initial = decode(proxy.state().await.map_err(|_| Error::Call)?)?;
    sender
        .send(Ok(initial.projection))
        .await
        .map_err(|_| Error::Publish)?;
    while let Some(signal) = changes.next().await {
        let arguments = signal.args().map_err(|_| Error::Protocol)?;
        let snapshot = decode(arguments.state().clone())?;
        if sender.send(Ok(snapshot.projection)).await.is_err() {
            return Ok(());
        }
    }
    Ok(())
}

fn decode(state: WireState) -> Result<Snapshot, Error> {
    let persistence_healthy = state.4;
    let mode_id = state.0.then(|| state.1.clone());
    let source = activation_source(&state).map_err(|_| Error::Protocol)?;
    Ok(Snapshot {
        projection: projection(&state).map_err(|_| Error::Protocol)?,
        mode_id,
        source,
        persistence_healthy,
    })
}

fn decode_settings(settings: WireSettings) -> Result<SettingsSnapshot, Error> {
    let configuration = decode_configuration(settings.0).map_err(|_| Error::Protocol)?;
    let state = decode(settings.1)?;
    if let Some(mode_id) = state.mode_id.as_deref() {
        let mode_id = rmac_focus::ModeId::parse(mode_id).map_err(|_| Error::Protocol)?;
        if configuration.mode(&mode_id).is_none() {
            return Err(Error::Protocol);
        }
    }
    if let Some(ActivationSource::Schedule(schedule_id)) = &state.source {
        let Some(schedule) = configuration
            .schedules()
            .find(|schedule| &schedule.id == schedule_id)
        else {
            return Err(Error::Protocol);
        };
        if state.mode_id.as_deref() != Some(schedule.mode.as_str()) {
            return Err(Error::Protocol);
        }
    }
    Ok(SettingsSnapshot {
        configuration,
        state,
    })
}

fn ensure_persisted(snapshot: Snapshot) -> Result<Snapshot, Error> {
    if snapshot.persistence_healthy {
        Ok(snapshot)
    } else {
        Err(Error::Persistence)
    }
}

fn call_error(error: zbus::Error) -> Error {
    match error {
        zbus::Error::MethodError(_, Some(detail), _) if detail == SCHEDULED_DISABLE_DETAIL => {
            Error::Scheduled
        }
        zbus::Error::MethodError(_, Some(detail), _)
            if detail == "Focus mode or duration is invalid"
                || detail == "Focus configuration is invalid" =>
        {
            Error::Invalid
        }
        _ => Error::Call,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Connect,
    Subscribe,
    Call,
    Scheduled,
    Invalid,
    Persistence,
    Protocol,
    Publish,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Scheduled => "scheduled Focus must be changed in Focus settings",
            Self::Invalid => "the Focus mode or duration is invalid",
            Self::Persistence => "Focus changed but could not be saved for the next sign-in",
            Self::Connect | Self::Subscribe | Self::Call | Self::Protocol | Self::Publish => {
                "the Focus authority is unavailable"
            }
        })
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_focus::{Mode, ModeId, Schedule, ScheduleId, Weekday};
    use std::collections::BTreeSet;

    #[test]
    fn decoding_exposes_persistence_health_without_private_state() {
        let snapshot = decode((
            true,
            "work".into(),
            "Work".into(),
            5_000,
            false,
            1,
            String::new(),
        ))
        .unwrap();
        assert!(snapshot.projection.enabled);
        assert_eq!(snapshot.projection.mode_name.as_deref(), Some("Work"));
        assert_eq!(snapshot.mode_id.as_deref(), Some("work"));
        assert_eq!(snapshot.source, Some(ActivationSource::Manual));
        assert!(!snapshot.persistence_healthy);
        assert!(!format!("{snapshot:?}").contains("work"));
    }

    #[test]
    fn protocol_constants_are_stable() {
        assert_eq!(BUS_NAME, INTERFACE_NAME);
        assert_eq!(OBJECT_PATH, "/org/rmac/Focus1");
    }

    #[test]
    fn persistence_failure_is_actionable_after_a_mutation() {
        let snapshot = decode((
            true,
            "work".into(),
            "Work".into(),
            0,
            false,
            1,
            String::new(),
        ))
        .unwrap();
        assert_eq!(ensure_persisted(snapshot), Err(Error::Persistence));
        assert!(Error::Persistence
            .to_string()
            .contains("could not be saved"));
    }

    #[test]
    fn settings_snapshot_debug_redacts_configuration_and_state_identity() {
        let mode_id = ModeId::parse("private-work-id").unwrap();
        let schedule_id = ScheduleId::parse("private-schedule-id").unwrap();
        let configuration = Config::new(
            vec![Mode::new(mode_id.clone(), "Private Work Name", BTreeSet::new(), false).unwrap()],
            vec![Schedule {
                id: schedule_id,
                mode: mode_id,
                days: BTreeSet::from([Weekday::Monday]),
                start_minute: 9 * 60,
                end_minute: 17 * 60,
                priority: 1,
                enabled: true,
            }],
        )
        .unwrap();
        let snapshot = decode_settings((
            encode_configuration(&configuration),
            (
                true,
                "private-work-id".into(),
                "Private Work Name".into(),
                0,
                true,
                2,
                "private-schedule-id".into(),
            ),
        ))
        .unwrap();
        let debug = format!("{snapshot:?}");
        assert!(!debug.contains("private-work-id"));
        assert!(!debug.contains("Private Work Name"));
        assert!(!debug.contains("private-schedule-id"));
        assert!(decode_settings((
            (Vec::new(), Vec::new()),
            (
                false,
                String::new(),
                String::new(),
                0,
                true,
                0,
                String::new(),
            )
        ))
        .is_err());
        assert!(decode_settings((
            encode_configuration(&configuration),
            (
                true,
                String::new(),
                "Missing ID".into(),
                0,
                true,
                1,
                String::new(),
            ),
        ))
        .is_err());
        assert!(decode_settings((
            encode_configuration(&configuration),
            (
                true,
                "private-work-id".into(),
                "Private Work Name".into(),
                0,
                true,
                2,
                "missing-schedule".into(),
            ),
        ))
        .is_err());
    }
}
