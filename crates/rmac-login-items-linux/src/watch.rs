//! Filesystem and systemd login-item watch authority.

use super::*;

#[cfg(target_os = "linux")]
pub async fn watch(
    sender: async_channel::Sender<rmac_login_items::WatchEvent>,
) -> Result<(), Error> {
    let environment = Environment::current();
    let _filesystem = match filesystem_watcher(&environment, sender.clone()) {
        Ok(watcher) => Some(watcher),
        Err(_) => {
            let _ = sender.try_send(rmac_login_items::WatchEvent::Unavailable);
            None
        }
    };
    loop {
        match watch_systemd_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => {
                let _ = sender.try_send(rmac_login_items::WatchEvent::Unavailable);
            }
        }
        async_io::Timer::after(RECONNECT_DELAY).await;
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn watch(
    sender: async_channel::Sender<rmac_login_items::WatchEvent>,
) -> Result<(), Error> {
    sender
        .send(rmac_login_items::WatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "login item watcher closed"))
}

#[cfg(target_os = "linux")]
pub(super) fn filesystem_watcher(
    environment: &Environment,
    sender: async_channel::Sender<rmac_login_items::WatchEvent>,
) -> Result<notify::RecommendedWatcher, Error> {
    use notify::{RecursiveMode, Watcher as _};

    let roots = filesystem_event_roots(environment);
    let filter_roots = roots.clone();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else {
            let _ = sender.try_send(rmac_login_items::WatchEvent::Unavailable);
            return;
        };
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
        }
        if event.paths.is_empty()
            || event
                .paths
                .iter()
                .any(|path| filter_roots.iter().any(|root| path.starts_with(root)))
        {
            let _ = sender.try_send(rmac_login_items::WatchEvent::Changed);
        }
    })
    .map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the login item filesystem watcher is unavailable",
        )
    })?;
    let mut watched = 0;
    for root in roots {
        let target = if root.is_dir() {
            root
        } else if let Some(parent) = root.parent().filter(|parent| parent.is_dir()) {
            parent.to_path_buf()
        } else {
            continue;
        };
        if watcher.watch(&target, RecursiveMode::Recursive).is_ok() {
            watched += 1;
        }
    }
    if watched == 0 {
        Err(Error::new(
            ErrorKind::Unavailable,
            "no login item directories could be watched",
        ))
    } else {
        Ok(watcher)
    }
}

#[cfg(target_os = "linux")]
pub(super) fn filesystem_event_roots(environment: &Environment) -> Vec<PathBuf> {
    let mut roots = vec![
        environment.config_home.join("autostart"),
        environment.config_home.join("systemd/user"),
        environment.data_home.join("systemd/user"),
    ];
    roots.extend(
        environment
            .config_dirs
            .iter()
            .map(|directory| directory.join("autostart")),
    );
    roots.sort();
    roots.dedup();
    roots
}

#[cfg(target_os = "linux")]
pub(super) async fn watch_systemd_once(
    sender: &async_channel::Sender<rmac_login_items::WatchEvent>,
) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = zbus::Connection::session().await.map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "systemd user event stream is unavailable",
        )
    })?;
    let unit_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.systemd1")
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid systemd signal sender"))?
        .path("/org/freedesktop/systemd1")
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid systemd manager path"))?
        .interface("org.freedesktop.systemd1.Manager")
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid systemd manager interface"))?
        .member("UnitFilesChanged")
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid systemd unit signal"))?
        .build();
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid D-Bus sender"))?
        .interface("org.freedesktop.DBus")
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid D-Bus interface"))?
        .member("NameOwnerChanged")
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid owner signal"))?
        .add_arg("org.freedesktop.systemd1")
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid systemd owner filter"))?
        .build();
    let mut units = MessageStream::for_match_rule(unit_rule, &connection, Some(8))
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not watch user unit files"))?
        .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule, &connection, Some(4))
        .await
        .map_err(|_| {
            Error::new(
                ErrorKind::Unavailable,
                "could not watch user manager restarts",
            )
        })?
        .fuse();
    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        let changed = futures_util::select! {
            message = units.next() => {
                message
                    .ok_or_else(|| Error::new(ErrorKind::Unavailable, "user unit stream ended"))?
                    .map_err(|_| Error::new(ErrorKind::Unavailable, "user unit stream failed"))?;
                true
            },
            message = owners.next() => systemd_owner_reappeared(message)?,
            _ = closed => return Ok(()),
        };
        if changed {
            let _ = sender.try_send(rmac_login_items::WatchEvent::Changed);
        }
    }
}

#[cfg(target_os = "linux")]
pub(super) fn systemd_owner_reappeared(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<bool, Error> {
    let message = message
        .ok_or_else(|| Error::new(ErrorKind::Unavailable, "user manager owner stream ended"))?
        .map_err(|_| Error::new(ErrorKind::Unavailable, "user manager owner stream failed"))?;
    let (name, _old_owner, new_owner): (String, String, String) = message
        .body()
        .deserialize()
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid user manager owner change"))?;
    Ok(owner_change_reappeared(&name, &new_owner))
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn owner_change_reappeared(name: &str, new_owner: &str) -> bool {
    name == "org.freedesktop.systemd1" && !new_owner.is_empty()
}
