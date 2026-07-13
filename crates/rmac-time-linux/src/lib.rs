//! Linux systemd-timedated adapter.

use rmac_time::{Error, ErrorKind, Service, Snapshot};

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemService;

impl Service for SystemService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        system_snapshot()
    }

    fn set_ntp(&self, enabled: bool) -> Result<Snapshot, Error> {
        system_set_ntp(enabled)?;
        self.snapshot()
    }

    fn set_timezone(&self, timezone: &str) -> Result<Snapshot, Error> {
        let snapshot = self.snapshot()?;
        snapshot.validate_timezone(timezone)?;
        system_set_timezone(timezone)?;
        self.snapshot()
    }
}

pub fn snapshot() -> Result<Snapshot, Error> {
    SystemService.snapshot()
}

pub fn set_ntp(enabled: bool) -> Result<Snapshot, Error> {
    SystemService.set_ntp(enabled)
}

pub fn set_timezone(timezone: &str) -> Result<Snapshot, Error> {
    SystemService.set_timezone(timezone)
}

#[cfg(target_os = "linux")]
const RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

#[cfg(target_os = "linux")]
pub async fn watch(sender: async_channel::Sender<rmac_time::WatchEvent>) -> Result<(), Error> {
    loop {
        match watch_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => {
                let _ = sender.try_send(rmac_time::WatchEvent::Unavailable);
            }
        }
        async_io::Timer::after(RECONNECT_DELAY).await;
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn watch(sender: async_channel::Sender<rmac_time::WatchEvent>) -> Result<(), Error> {
    sender
        .send(rmac_time::WatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "date and time watcher closed"))
}

#[cfg(target_os = "linux")]
async fn watch_once(sender: &async_channel::Sender<rmac_time::WatchEvent>) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = zbus::Connection::system().await.map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the system date and time event stream is unavailable",
        )
    })?;
    let properties_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .path("/org/freedesktop/timedate1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid timedated event path"))?
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
        .add_arg("org.freedesktop.timedate1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid timedated owner filter"))?
        .build();
    let mut properties = MessageStream::for_match_rule(properties_rule, &connection, Some(8))
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not watch timedated changes"))?
        .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule, &connection, Some(4))
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not watch timedated restarts"))?
        .fuse();

    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        let changed = futures_util::select! {
            message = properties.next() => {
                message
                    .ok_or_else(|| Error::new(ErrorKind::Unavailable, "timedated event stream ended"))?
                    .map_err(|_| Error::new(ErrorKind::Unavailable, "timedated event stream failed"))?;
                true
            },
            message = owners.next() => {
                owner_reappeared(message)?
            },
            _ = closed => return Ok(()),
        };
        if changed {
            let _ = sender.try_send(rmac_time::WatchEvent::Changed);
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
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid timedated owner change"))?;
    Ok(owner_change_reappeared(&name, &new_owner))
}

#[cfg(any(target_os = "linux", test))]
fn owner_change_reappeared(name: &str, new_owner: &str) -> bool {
    name == "org.freedesktop.timedate1" && !new_owner.is_empty()
}

#[cfg(target_os = "linux")]
fn system_snapshot() -> Result<Snapshot, Error> {
    let connection = system_connection()?;
    let proxy = timedate_proxy(&connection)?;
    let timezone = property::<String>(&proxy, "Timezone")?;
    let local_rtc = property::<bool>(&proxy, "LocalRTC")?;
    let can_ntp = property::<bool>(&proxy, "CanNTP")?;
    let ntp_enabled = property::<bool>(&proxy, "NTP")?;
    let synchronized = property::<bool>(&proxy, "NTPSynchronized")?;
    let time_usec = property::<u64>(&proxy, "TimeUSec")?;
    let timezones = proxy
        .call::<_, _, Vec<String>>("ListTimezones", &())
        .map_err(|_| Error::new(ErrorKind::Protocol, "could not list system time zones"))?;
    let (timezones, timezones_truncated) = rmac_time::normalize_timezones(timezones);
    Ok(Snapshot {
        timezone,
        local_rtc,
        can_ntp,
        ntp_enabled,
        synchronized,
        time_usec,
        timezones,
        timezones_truncated,
    })
}

#[cfg(not(target_os = "linux"))]
fn system_snapshot() -> Result<Snapshot, Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "date and time settings are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
fn system_set_ntp(enabled: bool) -> Result<(), Error> {
    let connection = system_connection()?;
    let proxy = timedate_proxy(&connection)?;
    proxy
        .call::<_, _, ()>("SetNTP", &(enabled, true))
        .map_err(mutation_error)
}

#[cfg(not(target_os = "linux"))]
fn system_set_ntp(_enabled: bool) -> Result<(), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "automatic time is available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
fn system_set_timezone(timezone: &str) -> Result<(), Error> {
    let connection = system_connection()?;
    let proxy = timedate_proxy(&connection)?;
    proxy
        .call::<_, _, ()>("SetTimezone", &(timezone, true))
        .map_err(mutation_error)
}

#[cfg(not(target_os = "linux"))]
fn system_set_timezone(_timezone: &str) -> Result<(), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "time-zone changes are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
fn system_connection() -> Result<zbus::blocking::Connection, Error> {
    zbus::blocking::Connection::system().map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the system date and time service is unavailable",
        )
    })
}

#[cfg(target_os = "linux")]
fn timedate_proxy(
    connection: &zbus::blocking::Connection,
) -> Result<zbus::blocking::Proxy<'_>, Error> {
    zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.timedate1",
        "/org/freedesktop/timedate1",
        "org.freedesktop.timedate1",
    )
    .map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the system date and time service is unavailable",
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
        assert!(!owner_change_reappeared("org.freedesktop.timedate1", ""));
        assert!(owner_change_reappeared(
            "org.freedesktop.timedate1",
            ":1.42"
        ));
        assert!(!owner_change_reappeared("org.example.Other", ":1.42"));
    }
}
