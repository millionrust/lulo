use rmac_sharing::{Error, ErrorKind};

#[cfg(target_os = "linux")]
const RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

#[cfg(target_os = "linux")]
pub async fn watch(sender: async_channel::Sender<rmac_sharing::WatchEvent>) -> Result<(), Error> {
    let _configuration = match configuration_watcher(sender.clone()) {
        Ok(watcher) => watcher,
        Err(_) => {
            let _ = sender.try_send(rmac_sharing::WatchEvent::Unavailable);
            None
        }
    };
    loop {
        match watch_systemd_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => {
                let _ = sender.try_send(rmac_sharing::WatchEvent::Unavailable);
            }
        }
        async_io::Timer::after(RECONNECT_DELAY).await;
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn watch(sender: async_channel::Sender<rmac_sharing::WatchEvent>) -> Result<(), Error> {
    sender
        .send(rmac_sharing::WatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "Sharing watcher closed"))
}

#[cfg(target_os = "linux")]
fn configuration_watcher(
    sender: async_channel::Sender<rmac_sharing::WatchEvent>,
) -> Result<Option<notify::RecommendedWatcher>, Error> {
    use notify::{RecursiveMode, Watcher as _};

    let roots = ["/etc/ufw", "/etc/samba"]
        .into_iter()
        .map(std::path::PathBuf::from)
        .filter(|root| root.is_dir())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Ok(None);
    }
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else {
            let _ = sender.try_send(rmac_sharing::WatchEvent::Unavailable);
            return;
        };
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
        }
        if event.paths.is_empty()
            || event
                .paths
                .iter()
                .any(|path| firewall_path_relevant(path) || samba_path_relevant(path))
        {
            let _ = sender.try_send(rmac_sharing::WatchEvent::Changed);
        }
    })
    .map_err(|error| Error::new(ErrorKind::Unavailable, error.to_string()))?;
    for root in roots {
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|error| Error::new(ErrorKind::Unavailable, error.to_string()))?;
    }
    Ok(Some(watcher))
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn firewall_path_relevant(path: &std::path::Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("ufw.conf" | "user.rules" | "user6.rules")
    ) || path.ancestors().any(|ancestor| {
        ancestor.file_name().and_then(|name| name.to_str()) == Some("applications.d")
    })
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn samba_path_relevant(path: &std::path::Path) -> bool {
    path.ancestors()
        .any(|ancestor| ancestor == std::path::Path::new("/etc/samba"))
        && path.extension().and_then(|extension| extension.to_str()) == Some("conf")
}

#[cfg(target_os = "linux")]
async fn watch_systemd_once(
    sender: &async_channel::Sender<rmac_sharing::WatchEvent>,
) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = zbus::Connection::system().await.map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "Sharing system event stream is unavailable",
        )
    })?;
    let properties_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.systemd1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid systemd sender"))?
        .interface("org.freedesktop.DBus.Properties")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid properties interface"))?
        .member("PropertiesChanged")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid properties signal"))?
        .add_arg("org.freedesktop.systemd1.Unit")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid unit property filter"))?
        .build();
    let files_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.systemd1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid systemd sender"))?
        .path("/org/freedesktop/systemd1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid systemd path"))?
        .interface("org.freedesktop.systemd1.Manager")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid manager interface"))?
        .member("UnitFilesChanged")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid unit-files signal"))?
        .build();
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid D-Bus sender"))?
        .interface("org.freedesktop.DBus")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid D-Bus interface"))?
        .member("NameOwnerChanged")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid owner signal"))?
        .add_arg("org.freedesktop.systemd1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid systemd owner filter"))?
        .build();
    let mut properties = MessageStream::for_match_rule(properties_rule, &connection, Some(16))
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not watch system units"))?
        .fuse();
    let mut files = MessageStream::for_match_rule(files_rule, &connection, Some(4))
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not watch unit files"))?
        .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule, &connection, Some(4))
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not watch systemd restarts"))?
        .fuse();
    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        let changed = futures_util::select! {
            message = properties.next() => stream_changed(message, "system unit")?,
            message = files.next() => stream_changed(message, "unit file")?,
            message = owners.next() => systemd_owner_reappeared(message)?,
            _ = closed => return Ok(()),
        };
        if changed {
            let _ = sender.try_send(rmac_sharing::WatchEvent::Changed);
        }
    }
}

#[cfg(target_os = "linux")]
fn stream_changed(
    message: Option<Result<zbus::Message, zbus::Error>>,
    name: &str,
) -> Result<bool, Error> {
    message
        .ok_or_else(|| Error::new(ErrorKind::Unavailable, format!("{name} stream ended")))?
        .map_err(|_| Error::new(ErrorKind::Unavailable, format!("{name} stream failed")))?;
    Ok(true)
}

#[cfg(target_os = "linux")]
fn systemd_owner_reappeared(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<bool, Error> {
    let message = message
        .ok_or_else(|| Error::new(ErrorKind::Unavailable, "systemd owner stream ended"))?
        .map_err(|_| Error::new(ErrorKind::Unavailable, "systemd owner stream failed"))?;
    let (name, _old_owner, new_owner): (String, String, String) = message
        .body()
        .deserialize()
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid systemd owner change"))?;
    Ok(owner_change_reappeared(&name, &new_owner))
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn owner_change_reappeared(name: &str, new_owner: &str) -> bool {
    name == "org.freedesktop.systemd1" && !new_owner.is_empty()
}
