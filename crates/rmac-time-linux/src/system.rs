use rmac_time::{Error, ErrorKind, Snapshot};

#[cfg(target_os = "linux")]
pub(crate) fn system_snapshot() -> Result<Snapshot, Error> {
    let connection = system_connection()?;
    let proxy = timedate_proxy(&connection)?;
    let timezone = property::<String>(&proxy, "Timezone")?;
    rmac_time::validate_timezone_syntax(&timezone).map_err(|_| {
        Error::new(
            ErrorKind::Protocol,
            "timedated returned an invalid current time zone",
        )
    })?;
    let local_rtc = property::<bool>(&proxy, "LocalRTC")?;
    let can_ntp = property::<bool>(&proxy, "CanNTP")?;
    let ntp_enabled = property::<bool>(&proxy, "NTP")?;
    let synchronized = property::<bool>(&proxy, "NTPSynchronized")?;
    let time_usec = property::<u64>(&proxy, "TimeUSec")?;
    let timezones = proxy
        .call::<_, _, Vec<String>>("ListTimezones", &())
        .map_err(|_| Error::new(ErrorKind::Protocol, "could not list system time zones"))?;
    if !timezones.iter().any(|candidate| candidate == &timezone) {
        return Err(Error::new(
            ErrorKind::Protocol,
            "the current time zone is missing from timedated's inventory",
        ));
    }
    let (mut timezones, timezones_truncated) = rmac_time::normalize_timezones(timezones);
    if !timezones.iter().any(|candidate| candidate == &timezone) {
        // The complete authority contained the current zone, but it sorted
        // beyond the bounded UI inventory. Retain it so an unchanged edit can
        // still validate without expanding the public bound.
        timezones.pop();
        timezones.push(timezone.clone());
        timezones.sort_unstable();
    }
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
pub(crate) fn system_snapshot() -> Result<Snapshot, Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "date and time settings are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn system_set_ntp(enabled: bool) -> Result<(), Error> {
    let connection = system_connection()?;
    let proxy = timedate_proxy(&connection)?;
    proxy
        .call::<_, _, ()>("SetNTP", &(enabled, true))
        .map_err(mutation_error)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn system_set_ntp(_enabled: bool) -> Result<(), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "automatic time is available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn system_set_timezone(timezone: &str) -> Result<(), Error> {
    let connection = system_connection()?;
    let proxy = timedate_proxy(&connection)?;
    proxy
        .call::<_, _, ()>("SetTimezone", &(timezone, true))
        .map_err(mutation_error)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn system_set_timezone(_timezone: &str) -> Result<(), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "time-zone changes are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn system_set_time(time_usec: u64) -> Result<(), Error> {
    let time_usec = i64::try_from(time_usec).map_err(|_| {
        Error::new(
            ErrorKind::InvalidTime,
            "the requested system time is outside timedated's supported range",
        )
    })?;
    let connection = system_connection()?;
    let proxy = timedate_proxy(&connection)?;
    proxy
        .call::<_, _, ()>("SetTime", &(time_usec, false, true))
        .map_err(mutation_error)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn system_set_time(_time_usec: u64) -> Result<(), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "manual system time is available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
fn system_connection() -> Result<zbus::blocking::Connection, Error> {
    rmac_dbus::system_blocking().map_err(|_| {
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
        Error::new(
            ErrorKind::Mutation,
            "timedated rejected the requested date and time change",
        )
    }
}
