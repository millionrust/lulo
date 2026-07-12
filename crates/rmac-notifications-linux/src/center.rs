//! Reconnecting client for Notification Center indicator state.

use async_channel::Sender;
use futures_util::StreamExt as _;
use rmac_notifications::Indicator;

#[cfg(test)]
use crate::service::{CENTER_BUS_NAME, CENTER_PATH};

#[zbus::proxy(
    interface = "org.rmac.NotificationCenter1",
    default_service = "org.rmac.NotificationCenter1",
    default_path = "/org/rmac/NotificationCenter1"
)]
trait Center {
    fn state(&self) -> zbus::Result<(u32, bool)>;

    #[zbus(signal)]
    fn changed(&self, unread_count: u32, has_urgent: bool) -> zbus::Result<()>;
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
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Notification Center is unavailable")
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
}
