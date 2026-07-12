//! Reconnecting client for Notification Center indicator state.

use async_channel::Sender;
use futures_util::StreamExt as _;
use rmac_notifications::{AppId, Indicator};
use rmac_notifications_store::{AppPolicy, LockPreview};

pub type WireAppPolicy = (bool, bool, bool, bool, bool, bool, u8);
const MAX_WIRE_APPLICATIONS: usize = 1_012;

pub fn encode_policy(policy: AppPolicy) -> WireAppPolicy {
    (
        policy.enabled,
        policy.banners,
        policy.sounds,
        policy.badges,
        policy.history,
        policy.urgent_through_focus,
        match policy.lock_preview {
            LockPreview::Show => 0,
            LockPreview::HideContent => 1,
            LockPreview::Hide => 2,
        },
    )
}

pub fn decode_policy(policy: WireAppPolicy) -> Result<AppPolicy, Error> {
    Ok(AppPolicy {
        enabled: policy.0,
        banners: policy.1,
        sounds: policy.2,
        badges: policy.3,
        history: policy.4,
        urgent_through_focus: policy.5,
        lock_preview: match policy.6 {
            0 => LockPreview::Show,
            1 => LockPreview::HideContent,
            2 => LockPreview::Hide,
            _ => return Err(Error::Protocol),
        },
    })
}

#[cfg(test)]
use crate::service::{CENTER_BUS_NAME, CENTER_PATH};

#[zbus::proxy(
    interface = "org.rmac.NotificationCenter1",
    default_service = "org.rmac.NotificationCenter1",
    default_path = "/org/rmac/NotificationCenter1"
)]
trait Center {
    fn state(&self) -> zbus::Result<(u32, bool)>;
    fn applications(&self) -> zbus::Result<Vec<(String, WireAppPolicy)>>;
    fn mark_read(&self, app_id: &str) -> zbus::Result<(u32, bool)>;
    fn clear(&self, app_id: &str) -> zbus::Result<(u32, bool)>;
    fn set_policy(&self, app_id: &str, policy: WireAppPolicy) -> zbus::Result<(u32, bool)>;

    #[zbus(signal)]
    fn changed(&self, unread_count: u32, has_urgent: bool) -> zbus::Result<()>;

    #[zbus(signal)]
    fn policies_changed(&self) -> zbus::Result<()>;
}

#[derive(Clone, Eq, PartialEq)]
pub struct ApplicationPolicy {
    pub app_id: String,
    pub policy: AppPolicy,
}

impl std::fmt::Debug for ApplicationPolicy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApplicationPolicy")
            .field("app_id", &"<redacted>")
            .field("policy", &self.policy)
            .finish()
    }
}

pub fn applications() -> Result<Vec<ApplicationPolicy>, Error> {
    let connection = zbus::blocking::Connection::session().map_err(|_| Error::Connect)?;
    let proxy = CenterProxyBlocking::new(&connection).map_err(|_| Error::Connect)?;
    let applications = proxy.applications().map_err(call_error)?;
    if applications.len() > MAX_WIRE_APPLICATIONS {
        return Err(Error::Protocol);
    }
    applications
        .into_iter()
        .map(|(app_id, policy)| {
            let app_id = AppId::parse(app_id).map_err(|_| Error::Protocol)?;
            Ok(ApplicationPolicy {
                app_id: app_id.as_str().to_owned(),
                policy: decode_policy(policy)?,
            })
        })
        .collect()
}

pub fn mark_read(app_id: Option<&str>) -> Result<Indicator, Error> {
    mutate(|proxy| proxy.mark_read(app_id.unwrap_or_default()))
}

pub fn clear(app_id: Option<&str>) -> Result<Indicator, Error> {
    mutate(|proxy| proxy.clear(app_id.unwrap_or_default()))
}

pub fn set_policy(app_id: &str, policy: AppPolicy) -> Result<Indicator, Error> {
    mutate(|proxy| proxy.set_policy(app_id, encode_policy(policy)))
}

fn mutate(
    operation: impl FnOnce(&CenterProxyBlocking<'_>) -> zbus::Result<(u32, bool)>,
) -> Result<Indicator, Error> {
    let connection = zbus::blocking::Connection::session().map_err(|_| Error::Connect)?;
    let proxy = CenterProxyBlocking::new(&connection).map_err(|_| Error::Connect)?;
    let (unread_count, has_urgent) = operation(&proxy).map_err(call_error)?;
    Ok(Indicator {
        unread_count,
        has_urgent,
    })
}

fn call_error(error: zbus::Error) -> Error {
    match error {
        zbus::Error::MethodError(_, Some(detail), _)
            if detail == "Notification Center changed but could not be saved" =>
        {
            Error::Persistence
        }
        zbus::Error::MethodError(_, Some(detail), _)
            if detail == "Notification Center request is invalid" =>
        {
            Error::Invalid
        }
        _ => Error::Call,
    }
}

pub async fn watch(sender: Sender<Result<Indicator, String>>) -> Result<(), Error> {
    loop {
        match watch_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => publish_error(&sender, Error::Stopped).await?,
            Err(error) => publish_error(&sender, error).await?,
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

async fn watch_once(sender: &Sender<Result<Indicator, String>>) -> Result<(), Error> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|_| Error::Connect)?;
    let proxy = CenterProxy::new(&connection)
        .await
        .map_err(|_| Error::Connect)?;
    let mut changes = proxy
        .receive_changed()
        .await
        .map_err(|_| Error::Subscribe)?;
    let (unread_count, has_urgent) = proxy.state().await.map_err(|_| Error::Call)?;
    sender
        .send(Ok(Indicator {
            unread_count,
            has_urgent,
        }))
        .await
        .map_err(|_| Error::Publish)?;
    while let Some(signal) = changes.next().await {
        let arguments = signal.args().map_err(|_| Error::Protocol)?;
        if sender
            .send(Ok(Indicator {
                unread_count: *arguments.unread_count(),
                has_urgent: *arguments.has_urgent(),
            }))
            .await
            .is_err()
        {
            return Ok(());
        }
    }
    Ok(())
}

async fn publish_error(
    sender: &Sender<Result<Indicator, String>>,
    error: Error,
) -> Result<(), Error> {
    sender
        .send(Err(error.to_string()))
        .await
        .map_err(|_| Error::Publish)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Connect,
    Subscribe,
    Call,
    Protocol,
    Publish,
    Stopped,
    Invalid,
    Persistence,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "The Notification Center change is invalid",
            Self::Persistence => "Notification Center changed but could not be saved",
            Self::Connect
            | Self::Subscribe
            | Self::Call
            | Self::Protocol
            | Self::Publish
            | Self::Stopped => "Notification Center is unavailable",
        })
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_identity_and_errors_expose_no_notification_content() {
        assert_eq!(CENTER_BUS_NAME, "org.rmac.NotificationCenter1");
        assert_eq!(CENTER_PATH, "/org/rmac/NotificationCenter1");
        assert_eq!(
            Error::Protocol.to_string(),
            "Notification Center is unavailable"
        );
    }

    #[test]
    fn app_policy_wire_round_trip_preserves_every_setting() {
        let policy = AppPolicy {
            enabled: false,
            banners: false,
            sounds: true,
            badges: false,
            history: true,
            urgent_through_focus: false,
            lock_preview: LockPreview::HideContent,
        };
        assert_eq!(decode_policy(encode_policy(policy)).unwrap(), policy);
        assert!(decode_policy((true, true, true, true, true, true, 3)).is_err());
        let application = ApplicationPolicy {
            app_id: "org.private.App".into(),
            policy,
        };
        assert!(!format!("{application:?}").contains("org.private.App"));
    }
}
