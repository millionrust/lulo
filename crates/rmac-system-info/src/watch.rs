#[cfg(any(target_os = "linux", test))]
use crate::WatchEvent;

#[cfg(target_os = "linux")]
use crate::{Error, ErrorKind};

#[cfg(target_os = "linux")]
const RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

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

#[cfg(target_os = "linux")]
async fn watch_once(sender: &async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = rmac_dbus::system().await.map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "watch system information",
            "the system hostname event stream is unavailable",
        )
    })?;
    let properties_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .path("/org/freedesktop/hostname1")
        .map_err(|_| watch_protocol_error("invalid hostname event path"))?
        .interface("org.freedesktop.DBus.Properties")
        .map_err(|_| watch_protocol_error("invalid properties interface"))?
        .member("PropertiesChanged")
        .map_err(|_| watch_protocol_error("invalid properties signal"))?
        .add_arg("org.freedesktop.hostname1")
        .map_err(|_| watch_protocol_error("invalid hostname interface filter"))?
        .build();
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|_| watch_protocol_error("invalid D-Bus sender"))?
        .interface("org.freedesktop.DBus")
        .map_err(|_| watch_protocol_error("invalid D-Bus interface"))?
        .member("NameOwnerChanged")
        .map_err(|_| watch_protocol_error("invalid owner-change signal"))?
        .add_arg("org.freedesktop.hostname1")
        .map_err(|_| watch_protocol_error("invalid hostname owner filter"))?
        .build();
    let mut properties = MessageStream::for_match_rule(properties_rule, &connection, Some(8))
        .await
        .map_err(|_| watch_protocol_error("could not watch hostname changes"))?
        .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule, &connection, Some(4))
        .await
        .map_err(|_| watch_protocol_error("could not watch hostname service restarts"))?
        .fuse();

    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        futures_util::select! {
            message = properties.next() => {
                message
                    .ok_or_else(|| watch_protocol_error("hostname event stream ended"))?
                    .map_err(|_| watch_protocol_error("hostname event stream failed"))?;
                let _ = sender.try_send(WatchEvent::Changed);
            },
            message = owners.next() => {
                // systemd-hostnamed is bus-activated and exits after a short
                // idle period as a matter of course (`Deactivated
                // successfully`, not a failure): the well-known name's owner
                // goes briefly empty, then a fresh instance reclaims it on
                // the next request. That owner-loss is routine and carries
                // no user-visible information, so it must not surface as
                // "unavailable" -- doing so turned an idle timer into a
                // recurring error banner. Only a name recovering (the
                // service coming back with a new owner) is worth an event,
                // to pick up anything that changed while it was down; a
                // stream that cannot be read at all still reaches the outer
                // reconnect loop below and reports Unavailable there.
                if let Some(event) = owner_watch_event(message)? {
                    let _ = sender.try_send(event);
                }
            },
            _ = closed => return Ok(()),
        };
    }
}

#[cfg(target_os = "linux")]
fn owner_watch_event(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<Option<WatchEvent>, Error> {
    let message = message
        .ok_or_else(|| watch_protocol_error("hostname owner stream ended"))?
        .map_err(|_| watch_protocol_error("hostname owner stream failed"))?;
    let (name, _old_owner, new_owner): (String, String, String) = message
        .body()
        .deserialize()
        .map_err(|_| watch_protocol_error("invalid hostname owner change"))?;
    Ok(owner_change_event(&name, &new_owner))
}

/// `None` means "nothing worth surfacing": either the signal was not about
/// `org.freedesktop.hostname1` (the match rule already filters this, but the
/// check stays cheap insurance), or the name simply lost its owner, which is
/// systemd-hostnamed's routine idle-exit rather than a failure. Only the name
/// regaining an owner produces an event, since that is the moment a change
/// made while the service was down could first be observed.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn owner_change_event(name: &str, new_owner: &str) -> Option<WatchEvent> {
    if name != "org.freedesktop.hostname1" || new_owner.is_empty() {
        return None;
    }
    Some(WatchEvent::Changed)
}

#[cfg(target_os = "linux")]
fn watch_protocol_error(detail: &'static str) -> Error {
    Error::new(ErrorKind::Unavailable, "watch system information", detail)
}
