//! Reconnecting client for Notification Center indicator state.

use async_channel::Sender;
use futures_util::StreamExt as _;
use rmac_notifications::{AppId, Content, Indicator, NotificationId, Priority};
use rmac_notifications_store::{AppPolicy, LockPreview};
use std::collections::BTreeSet;

pub type WireAppPolicy = (bool, bool, bool, bool, bool, bool, u8);
pub type WireHistoryRecord = (u32, String, String, String, u8, bool);
pub type WireSnapshot = (Vec<WireHistoryRecord>, Vec<(String, WireAppPolicy)>);
const MAX_WIRE_APPLICATIONS: usize = 1_012;
const MAX_WIRE_HISTORY: usize = 500;

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

pub(crate) fn encode_priority(priority: Priority) -> u8 {
    match priority {
        Priority::Low => 0,
        Priority::Normal => 1,
        Priority::High => 2,
        Priority::Urgent => 3,
    }
}

fn decode_priority(priority: u8) -> Result<Priority, Error> {
    match priority {
        0 => Ok(Priority::Low),
        1 => Ok(Priority::Normal),
        2 => Ok(Priority::High),
        3 => Ok(Priority::Urgent),
        _ => Err(Error::Protocol),
    }
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
    fn snapshot(&self) -> zbus::Result<WireSnapshot>;
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

#[derive(Clone, Eq, PartialEq)]
pub struct HistoryRecord {
    pub id: NotificationId,
    pub app_id: String,
    pub content: Content,
    pub priority: Priority,
    pub unread: bool,
}

impl std::fmt::Debug for HistoryRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HistoryRecord")
            .field("id", &self.id)
            .field("app_id", &"<redacted>")
            .field("content", &"<redacted>")
            .field("priority", &self.priority)
            .field("unread", &self.unread)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Snapshot {
    pub records: Vec<HistoryRecord>,
    pub applications: Vec<ApplicationPolicy>,
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Snapshot")
            .field(
                "records",
                &format_args!("<{} redacted records>", self.records.len()),
            )
            .field(
                "applications",
                &format_args!("<{} redacted apps>", self.applications.len()),
            )
            .finish()
    }
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
    decode_applications(proxy.applications().map_err(call_error)?)
}

pub fn snapshot() -> Result<Snapshot, Error> {
    let connection = zbus::blocking::Connection::session().map_err(|_| Error::Connect)?;
    let proxy = CenterProxyBlocking::new(&connection).map_err(|_| Error::Connect)?;
    let (history, applications) = proxy.snapshot().map_err(call_error)?;
    decode_snapshot(history, applications)
}

fn decode_snapshot(
    history: Vec<WireHistoryRecord>,
    applications: Vec<(String, WireAppPolicy)>,
) -> Result<Snapshot, Error> {
    Ok(Snapshot {
        records: decode_history(history)?,
        applications: decode_applications(applications)?,
    })
}

fn decode_history(history: Vec<WireHistoryRecord>) -> Result<Vec<HistoryRecord>, Error> {
    if history.len() > MAX_WIRE_HISTORY {
        return Err(Error::Protocol);
    }
    let mut seen = BTreeSet::new();
    history
        .into_iter()
        .map(|(id, app_id, title, body, priority, unread)| {
            let id = NotificationId::from_protocol(id).ok_or(Error::Protocol)?;
            if !seen.insert(id) {
                return Err(Error::Protocol);
            }
            let app_id = AppId::parse(app_id).map_err(|_| Error::Protocol)?;
            let content = Content::new(title, body).map_err(|_| Error::Protocol)?;
            Ok(HistoryRecord {
                id,
                app_id: app_id.as_str().to_owned(),
                content,
                priority: decode_priority(priority)?,
                unread,
            })
        })
        .collect()
}

fn decode_applications(
    applications: Vec<(String, WireAppPolicy)>,
) -> Result<Vec<ApplicationPolicy>, Error> {
    if applications.len() > MAX_WIRE_APPLICATIONS {
        return Err(Error::Protocol);
    }
    let mut seen = BTreeSet::new();
    let mut applications = applications
        .into_iter()
        .map(|(app_id, policy)| {
            let app_id = AppId::parse(app_id).map_err(|_| Error::Protocol)?;
            if !seen.insert(app_id.clone()) {
                return Err(Error::Protocol);
            }
            Ok(ApplicationPolicy {
                app_id: app_id.as_str().to_owned(),
                policy: decode_policy(policy)?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    applications.sort_by(|left, right| left.app_id.cmp(&right.app_id));
    Ok(applications)
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

pub async fn watch_applications(
    sender: Sender<Result<Vec<ApplicationPolicy>, String>>,
) -> Result<(), Error> {
    loop {
        match watch_applications_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => publish_applications_error(&sender, Error::Stopped).await?,
            Err(error) => publish_applications_error(&sender, error).await?,
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

pub async fn watch_snapshot(sender: Sender<Result<Snapshot, String>>) -> Result<(), Error> {
    loop {
        match watch_snapshot_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => publish_snapshot_error(&sender, Error::Stopped).await?,
            Err(error) => publish_snapshot_error(&sender, error).await?,
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

async fn watch_snapshot_once(sender: &Sender<Result<Snapshot, String>>) -> Result<(), Error> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|_| Error::Connect)?;
    let proxy = CenterProxy::new(&connection)
        .await
        .map_err(|_| Error::Connect)?;
    let center_changes = proxy
        .receive_changed()
        .await
        .map_err(|_| Error::Subscribe)?;
    publish_snapshot(&proxy, sender).await?;
    futures_util::pin_mut!(center_changes);
    loop {
        if center_changes.next().await.is_none() {
            return Ok(());
        }
        publish_snapshot(&proxy, sender).await?;
    }
}

async fn publish_snapshot(
    proxy: &CenterProxy<'_>,
    sender: &Sender<Result<Snapshot, String>>,
) -> Result<(), Error> {
    let (history, applications) = proxy.snapshot().await.map_err(call_error)?;
    let snapshot = decode_snapshot(history, applications)?;
    sender.send(Ok(snapshot)).await.map_err(|_| Error::Publish)
}

async fn publish_snapshot_error(
    sender: &Sender<Result<Snapshot, String>>,
    error: Error,
) -> Result<(), Error> {
    sender
        .send(Err(error.to_string()))
        .await
        .map_err(|_| Error::Publish)
}

async fn watch_applications_once(
    sender: &Sender<Result<Vec<ApplicationPolicy>, String>>,
) -> Result<(), Error> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|_| Error::Connect)?;
    let proxy = CenterProxy::new(&connection)
        .await
        .map_err(|_| Error::Connect)?;
    let policy_changes = proxy
        .receive_policies_changed()
        .await
        .map_err(|_| Error::Subscribe)?;
    let center_changes = proxy
        .receive_changed()
        .await
        .map_err(|_| Error::Subscribe)?;
    publish_applications(&proxy, sender).await?;
    futures_util::pin_mut!(policy_changes, center_changes);
    loop {
        let policy_change = futures_util::FutureExt::fuse(policy_changes.next());
        let center_change = futures_util::FutureExt::fuse(center_changes.next());
        futures_util::pin_mut!(policy_change, center_change);
        let open = futures_util::select! {
            change = policy_change => change.is_some(),
            change = center_change => change.is_some(),
        };
        if !open {
            return Ok(());
        }
        publish_applications(&proxy, sender).await?;
    }
}

async fn publish_applications(
    proxy: &CenterProxy<'_>,
    sender: &Sender<Result<Vec<ApplicationPolicy>, String>>,
) -> Result<(), Error> {
    let applications = decode_applications(proxy.applications().await.map_err(call_error)?)?;
    sender
        .send(Ok(applications))
        .await
        .map_err(|_| Error::Publish)
}

async fn publish_applications_error(
    sender: &Sender<Result<Vec<ApplicationPolicy>, String>>,
    error: Error,
) -> Result<(), Error> {
    sender
        .send(Err(error.to_string()))
        .await
        .map_err(|_| Error::Publish)
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

    #[test]
    fn application_decoder_is_bounded_unique_and_deterministic() {
        let policy = encode_policy(AppPolicy::default());
        let applications = decode_applications(vec![
            ("org.example.Zed".into(), policy),
            ("org.example.Alpha".into(), policy),
        ])
        .unwrap();
        assert_eq!(applications[0].app_id, "org.example.Alpha");
        assert_eq!(applications[1].app_id, "org.example.Zed");
        assert!(decode_applications(vec![
            ("org.example.Duplicate".into(), policy),
            ("org.example.Duplicate".into(), policy),
        ])
        .is_err());
        assert!(decode_applications(vec![("".into(), policy)]).is_err());
        assert!(decode_applications(vec![
            ("org.example.TooMany".into(), policy);
            MAX_WIRE_APPLICATIONS + 1
        ])
        .is_err());
    }

    #[test]
    fn history_decoder_revalidates_content_and_redacts_debug_output() {
        let records = decode_history(vec![(
            7,
            "org.example.Private".into(),
            "Private title".into(),
            "Private body".into(),
            encode_priority(Priority::Urgent),
            true,
        )])
        .unwrap();
        assert_eq!(records[0].id.get(), 7);
        assert_eq!(records[0].priority, Priority::Urgent);
        assert!(records[0].unread);
        let debug = format!("{:?}", records[0]);
        assert!(!debug.contains("Private"));
        assert!(!debug.contains("org.example"));

        assert!(decode_history(vec![(
            0,
            "org.example.Invalid".into(),
            "Title".into(),
            String::new(),
            1,
            false,
        )])
        .is_err());
        assert!(decode_history(vec![(
            1,
            "org.example.Invalid".into(),
            "Title".into(),
            String::new(),
            9,
            false,
        )])
        .is_err());
        assert!(decode_history(vec![
            (
                1,
                "org.example.App".into(),
                "One".into(),
                String::new(),
                1,
                false
            ),
            (
                1,
                "org.example.App".into(),
                "Two".into(),
                String::new(),
                1,
                false
            ),
        ])
        .is_err());
    }
}
