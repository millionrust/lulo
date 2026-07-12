//! Typed clients for the session-owned Focus authority.

use async_channel::Sender;
use futures_util::StreamExt as _;
use rmac_focus_runtime::Projection;

use crate::service::{projection, WireState, SCHEDULED_DISABLE_DETAIL};

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

    #[zbus(signal)]
    fn changed(&self, state: WireState) -> zbus::Result<()>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub projection: Projection,
    pub persistence_healthy: bool,
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
    Ok(Snapshot {
        projection: projection(&state).map_err(|_| Error::Protocol)?,
        persistence_healthy,
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
            if detail == "Focus mode or duration is invalid" =>
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

    #[test]
    fn decoding_exposes_persistence_health_without_private_state() {
        let snapshot = decode((true, "work".into(), "Work".into(), 5_000, false)).unwrap();
        assert!(snapshot.projection.enabled);
        assert_eq!(snapshot.projection.mode_name.as_deref(), Some("Work"));
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
        let snapshot = decode((true, "work".into(), "Work".into(), 0, false)).unwrap();
        assert_eq!(ensure_persisted(snapshot), Err(Error::Persistence));
        assert!(Error::Persistence
            .to_string()
            .contains("could not be saved"));
    }
}
