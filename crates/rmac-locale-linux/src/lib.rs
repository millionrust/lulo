//! Linux systemd-localed adapter.

use rmac_locale::{Error, ErrorKind, Service, Snapshot};

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemService;

impl Service for SystemService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        system_snapshot()
    }

    fn set_locale(&self, assignments: &[String]) -> Result<Snapshot, Error> {
        let current = self.snapshot()?;
        let normalized = rmac_locale::normalize_assignments(assignments.to_vec())?;
        if let Some(language) = normalized
            .iter()
            .find(|assignment| assignment.key == "LANG")
        {
            current.validate_installed(&language.value)?;
        }
        let encoded = normalized
            .iter()
            .map(rmac_locale::Assignment::encoded)
            .collect::<Vec<_>>();
        system_set_locale(&encoded)?;
        self.snapshot()
    }
}

pub fn snapshot() -> Result<Snapshot, Error> {
    SystemService.snapshot()
}

pub fn set_locale(assignments: &[String]) -> Result<Snapshot, Error> {
    SystemService.set_locale(assignments)
}

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

    let connection = zbus::Connection::system().await.map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the system locale event stream is unavailable",
        )
    })?;
    let properties_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .path("/org/freedesktop/locale1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid localed event path"))?
        .interface("org.freedesktop.DBus.Properties")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid properties interface"))?
        .member("PropertiesChanged")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid properties signal"))?
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
fn owner_change_reappeared(name: &str, new_owner: &str) -> bool {
    name == "org.freedesktop.locale1" && !new_owner.is_empty()
}

#[cfg(target_os = "linux")]
fn system_snapshot() -> Result<Snapshot, Error> {
    let connection = system_connection()?;
    let proxy = locale_proxy(&connection)?;
    let locale = rmac_locale::normalize_assignments(property(&proxy, "Locale")?)?;
    let (installed_locales, installed_locales_truncated) = installed_locales()?;
    Ok(Snapshot {
        locale,
        installed_locales,
        installed_locales_truncated,
        x11_layout: property(&proxy, "X11Layout")?,
        x11_model: property(&proxy, "X11Model")?,
        x11_variant: property(&proxy, "X11Variant")?,
        x11_options: property(&proxy, "X11Options")?,
        console_keymap: property(&proxy, "VConsoleKeymap")?,
    })
}

#[cfg(not(target_os = "linux"))]
fn system_snapshot() -> Result<Snapshot, Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "language and region settings are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
fn system_set_locale(assignments: &[String]) -> Result<(), Error> {
    rmac_locale::normalize_assignments(assignments.to_vec())?;
    let connection = system_connection()?;
    let proxy = locale_proxy(&connection)?;
    proxy
        .call::<_, _, ()>("SetLocale", &(assignments, true))
        .map_err(mutation_error)
}

#[cfg(not(target_os = "linux"))]
fn system_set_locale(_assignments: &[String]) -> Result<(), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "locale changes are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
fn installed_locales() -> Result<(Vec<String>, bool), Error> {
    let output = std::process::Command::new("locale")
        .arg("-a")
        .output()
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not list installed locales"))?;
    if !output.status.success() {
        return Err(Error::new(
            ErrorKind::Unavailable,
            "could not list installed locales",
        ));
    }
    let output = String::from_utf8(output.stdout)
        .map_err(|_| Error::new(ErrorKind::Protocol, "installed locale list is not UTF-8"))?;
    Ok(rmac_locale::normalize_installed_locales(
        output.lines().map(str::to_string).collect(),
    ))
}

#[cfg(target_os = "linux")]
fn system_connection() -> Result<zbus::blocking::Connection, Error> {
    zbus::blocking::Connection::system().map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the system locale service is unavailable",
        )
    })
}

#[cfg(target_os = "linux")]
fn locale_proxy(
    connection: &zbus::blocking::Connection,
) -> Result<zbus::blocking::Proxy<'_>, Error> {
    zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.locale1",
        "/org/freedesktop/locale1",
        "org.freedesktop.locale1",
    )
    .map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the system locale service is unavailable",
        )
    })
}

#[cfg(target_os = "linux")]
fn property<T>(proxy: &zbus::blocking::Proxy<'_>, name: &str) -> Result<T, Error>
where
    T: TryFrom<zbus::zvariant::OwnedValue>,
    T::Error: Into<zbus::Error>,
{
    proxy
        .get_property(name)
        .map_err(|_| Error::new(ErrorKind::Protocol, format!("could not read {name}")))
}

#[cfg(target_os = "linux")]
fn mutation_error(error: zbus::Error) -> Error {
    let detail = error.to_string();
    let lowercase = detail.to_ascii_lowercase();
    if lowercase.contains("accessdenied")
        || lowercase.contains("not authorized")
        || lowercase.contains("authentication")
        || lowercase.contains("polkit")
        || lowercase.contains("policykit")
    {
        Error::new(
            ErrorKind::Authorization,
            "authorization was denied or cancelled",
        )
    } else {
        Error::new(ErrorKind::Mutation, detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_exit_is_ignored_but_reappearance_refreshes() {
        assert!(!owner_change_reappeared("org.freedesktop.locale1", ""));
        assert!(owner_change_reappeared("org.freedesktop.locale1", ":1.42"));
        assert!(!owner_change_reappeared("org.example.Other", ":1.42"));
    }
}
