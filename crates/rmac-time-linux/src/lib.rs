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
