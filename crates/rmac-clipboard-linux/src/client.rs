//! Typed client for the session clipboard history.

use async_channel::Sender;
use futures_util::StreamExt as _;
use rmac_clipboard::{Item, WireEntry};

use crate::Error;

#[zbus::proxy(
    interface = "org.rmac.Clipboard1",
    default_service = "org.rmac.Clipboard1",
    default_path = "/org/rmac/Clipboard1"
)]
trait Clipboard {
    fn enabled(&self) -> zbus::Result<bool>;
    fn set_enabled(&self, enabled: bool) -> zbus::Result<()>;
    fn items(&self) -> zbus::Result<Vec<WireEntry>>;
    fn copy(&self, id: u64) -> zbus::Result<()>;
    fn remove(&self, id: u64) -> zbus::Result<()>;
    fn clear(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn changed(&self) -> zbus::Result<()>;
}

/// What the Clipboard view shows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Snapshot {
    /// The user has not allowed clipboard history.
    Disabled,
    /// Newest first.
    Items(Vec<Item>),
}

async fn proxy() -> Result<ClipboardProxy<'static>, Error> {
    let connection = zbus::Connection::session().await.map_err(|_| Error::Bus)?;
    ClipboardProxy::new(&connection)
        .await
        .map_err(|_| Error::Bus)
}

async fn snapshot_with(proxy: &ClipboardProxy<'_>) -> Result<Snapshot, Error> {
    if !proxy.enabled().await.map_err(|_| Error::Bus)? {
        return Ok(Snapshot::Disabled);
    }
    let items = proxy.items().await.map_err(|_| Error::Bus)?;
    items
        .into_iter()
        .map(|wire| rmac_clipboard::decode(wire).ok_or(Error::Protocol))
        .collect::<Result<Vec<_>, _>>()
        .map(Snapshot::Items)
}

pub async fn snapshot() -> Result<Snapshot, Error> {
    snapshot_with(&proxy().await?).await
}

pub async fn set_enabled(enabled: bool) -> Result<(), Error> {
    proxy()
        .await?
        .set_enabled(enabled)
        .await
        .map_err(|_| Error::Bus)
}

pub async fn copy(id: u64) -> Result<(), Error> {
    proxy().await?.copy(id).await.map_err(|_| Error::Clipboard)
}

pub async fn remove(id: u64) -> Result<(), Error> {
    proxy().await?.remove(id).await.map_err(|_| Error::Bus)
}

pub async fn clear() -> Result<(), Error> {
    proxy().await?.clear().await.map_err(|_| Error::Bus)
}

/// Send the current snapshot, then a fresh one after every change, until
/// the receiver closes. Returns an error when the service goes away.
pub async fn watch(sender: Sender<Result<Snapshot, Error>>) -> Result<(), Error> {
    let proxy = proxy().await?;
    let mut changes = proxy.receive_changed().await.map_err(|_| Error::Bus)?;
    if sender.send(snapshot_with(&proxy).await).await.is_err() {
        return Ok(());
    }
    while changes.next().await.is_some() {
        if sender.send(snapshot_with(&proxy).await).await.is_err() {
            return Ok(());
        }
    }
    Err(Error::Bus)
}
