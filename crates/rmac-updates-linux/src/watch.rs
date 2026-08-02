use rmac_updates::{Error, ErrorKind, WatchEvent};

#[cfg(any(target_os = "linux", test))]
use crate::api::PACKAGEKIT_DESTINATION;
#[cfg(target_os = "linux")]
use crate::api::{PACKAGEKIT_INTERFACE, PACKAGEKIT_PATH, RECONNECT_DELAY};
#[cfg(target_os = "linux")]
use crate::transaction::protocol_error;

#[cfg(target_os = "linux")]
pub async fn watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    loop {
        match watch_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => {
                let _ = sender.try_send(WatchEvent::Unavailable);
            }
        }
        async_io::Timer::after(RECONNECT_DELAY).await;
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    sender.send(WatchEvent::Unavailable).await.map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the software-update event receiver closed",
        )
    })
}
#[cfg(target_os = "linux")]
async fn watch_once(sender: &async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = zbus::Connection::system().await.map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the PackageKit event stream is unavailable",
        )
    })?;
    let updates_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .path(PACKAGEKIT_PATH)
        .map_err(|_| protocol_error("invalid PackageKit event path"))?
        .interface(PACKAGEKIT_INTERFACE)
        .map_err(|_| protocol_error("invalid PackageKit event interface"))?
        .build();
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|_| protocol_error("invalid D-Bus sender"))?
        .interface("org.freedesktop.DBus")
        .map_err(|_| protocol_error("invalid D-Bus interface"))?
        .member("NameOwnerChanged")
        .map_err(|_| protocol_error("invalid owner-change signal"))?
        .add_arg(PACKAGEKIT_DESTINATION)
        .map_err(|_| protocol_error("invalid PackageKit owner filter"))?
        .build();
    let mut updates = MessageStream::for_match_rule(updates_rule, &connection, Some(16))
        .await
        .map_err(|_| protocol_error("could not watch PackageKit changes"))?
        .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule, &connection, Some(4))
        .await
        .map_err(|_| protocol_error("could not watch PackageKit restarts"))?
        .fuse();

    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        let event = futures_util::select! {
            message = updates.next() => update_watch_event(message)?,
            message = owners.next() => owner_watch_event(message)?,
            _ = closed => return Ok(()),
        };
        if let Some(event) = event {
            let _ = sender.try_send(event);
        }
    }
}

#[cfg(target_os = "linux")]
fn update_watch_event(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<Option<WatchEvent>, Error> {
    let message = message
        .ok_or_else(|| protocol_error("PackageKit event stream ended"))?
        .map_err(|_| protocol_error("PackageKit event stream failed"))?;
    let member = message
        .header()
        .member()
        .map(|member| member.as_str().to_owned());
    Ok(matches!(
        member.as_deref(),
        Some("UpdatesChanged" | "RepoListChanged" | "RestartSchedule")
    )
    .then_some(WatchEvent::Changed))
}

#[cfg(target_os = "linux")]
fn owner_watch_event(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<Option<WatchEvent>, Error> {
    let message = message
        .ok_or_else(|| protocol_error("PackageKit owner stream ended"))?
        .map_err(|_| protocol_error("PackageKit owner stream failed"))?;
    let (name, _old_owner, new_owner): (String, String, String) = message
        .body()
        .deserialize()
        .map_err(|_| protocol_error("invalid PackageKit owner change"))?;
    Ok(packagekit_owner_event(&name, &new_owner))
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn packagekit_owner_event(name: &str, new_owner: &str) -> Option<WatchEvent> {
    (name == PACKAGEKIT_DESTINATION && !new_owner.is_empty()).then_some(WatchEvent::Changed)
}
