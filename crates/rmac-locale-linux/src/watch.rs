use rmac_locale::{Error, ErrorKind};

#[cfg(target_os = "linux")]
const RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

#[cfg(target_os = "linux")]
pub async fn watch(sender: async_channel::Sender<rmac_locale::WatchEvent>) -> Result<(), Error> {
    loop {
        match watch_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => {
                let _ = sender.try_send(rmac_locale::WatchEvent::Unavailable);
            }
        }
        async_io::Timer::after(RECONNECT_DELAY).await;
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn watch(sender: async_channel::Sender<rmac_locale::WatchEvent>) -> Result<(), Error> {
    sender
        .send(rmac_locale::WatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "language and region watcher closed"))
}

#[cfg(target_os = "linux")]
async fn watch_once(sender: &async_channel::Sender<rmac_locale::WatchEvent>) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = rmac_dbus::system().await.map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the system locale event stream is unavailable",
        )
    })?;
    let properties_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.locale1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid localed signal sender"))?
        .path("/org/freedesktop/locale1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid localed event path"))?
        .interface("org.freedesktop.DBus.Properties")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid properties interface"))?
        .member("PropertiesChanged")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid properties signal"))?
        .add_arg("org.freedesktop.locale1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid localed property filter"))?
        .build();
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid D-Bus sender"))?
        .interface("org.freedesktop.DBus")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid D-Bus interface"))?
        .member("NameOwnerChanged")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid owner-change signal"))?
        .add_arg("org.freedesktop.locale1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid localed owner filter"))?
        .build();
    let mut properties = MessageStream::for_match_rule(properties_rule, &connection, Some(8))
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not watch localed changes"))?
        .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule, &connection, Some(4))
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not watch localed restarts"))?
        .fuse();

    // The subscription is live before this refresh hint is published. A
    // consumer can now read a snapshot without losing a change between its
    // initial read and signal subscription.
    if sender.send(rmac_locale::WatchEvent::Changed).await.is_err() {
        return Ok(());
    }

    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        let changed = futures_util::select! {
            message = properties.next() => {
                message
                    .ok_or_else(|| Error::new(ErrorKind::Unavailable, "localed event stream ended"))?
                    .map_err(|_| Error::new(ErrorKind::Unavailable, "localed event stream failed"))?;
                true
            },
            message = owners.next() => owner_reappeared(message)?,
            _ = closed => return Ok(()),
        };
        if changed {
            let _ = sender.try_send(rmac_locale::WatchEvent::Changed);
        }
    }
}

#[cfg(target_os = "linux")]
fn owner_reappeared(message: Option<Result<zbus::Message, zbus::Error>>) -> Result<bool, Error> {
    let message = message
        .ok_or_else(|| Error::new(ErrorKind::Unavailable, "D-Bus owner stream ended"))?
        .map_err(|_| Error::new(ErrorKind::Unavailable, "D-Bus owner stream failed"))?;
    let (name, _old_owner, new_owner): (String, String, String) = message
        .body()
        .deserialize()
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid localed owner change"))?;
    Ok(owner_change_reappeared(&name, &new_owner))
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn owner_change_reappeared(name: &str, new_owner: &str) -> bool {
    name == "org.freedesktop.locale1" && !new_owner.is_empty()
}
