//! Linux systemd-localed adapter.

use rmac_locale::{Error, ErrorKind, Service, Snapshot};

#[cfg(target_os = "linux")]
use rmac_locale::FormatPreview;

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
    let (format_preview, format_preview_error) = match format_preview(&locale) {
        Ok(preview) => (Some(preview), None),
        Err(error) => (None, Some(error.to_string())),
    };
    Ok(Snapshot {
        locale,
        installed_locales,
        installed_locales_truncated,
        format_preview,
        format_preview_error,
        x11_layout: property(&proxy, "X11Layout")?,
        x11_model: property(&proxy, "X11Model")?,
        x11_variant: property(&proxy, "X11Variant")?,
        x11_options: property(&proxy, "X11Options")?,
        console_keymap: property(&proxy, "VConsoleKeymap")?,
    })
}

#[cfg(target_os = "linux")]
fn format_preview(locale: &[rmac_locale::Assignment]) -> Result<FormatPreview, Error> {
    let language = assignment(locale, "LANG").unwrap_or("C");
    let date_locale = assignment(locale, "LC_TIME").unwrap_or(language);
    let number_locale = assignment(locale, "LC_NUMERIC").unwrap_or(language);
    let currency_locale = assignment(locale, "LC_MONETARY").unwrap_or(language);

    let date = NativeLocale::new(date_locale)?;
    let number = NativeLocale::new(number_locale)?;
    let currency = NativeLocale::new(currency_locale)?;
    let decimal = number.langinfo(libc::RADIXCHAR)?;
    let thousands = number.langinfo(libc::THOUSEP)?;
    let number = grouped_number(&thousands, &decimal);
    Ok(FormatPreview {
        date_time: date.date_time()?,
        number,
        currency: currency.currency()?,
    })
}

#[cfg(target_os = "linux")]
fn assignment<'a>(locale: &'a [rmac_locale::Assignment], key: &str) -> Option<&'a str> {
    locale
        .iter()
        .find(|assignment| assignment.key == key)
        .map(|assignment| assignment.value.as_str())
}

#[cfg(target_os = "linux")]
struct NativeLocale(libc::locale_t);

#[cfg(target_os = "linux")]
impl NativeLocale {
    fn new(name: &str) -> Result<Self, Error> {
        let name = std::ffi::CString::new(name)
            .map_err(|_| Error::new(ErrorKind::Protocol, "locale name contains an invalid byte"))?;
        // SAFETY: `name` is a live NUL-terminated string, the base locale is
        // null as required for a new object, and the returned handle is owned
        // by this RAII wrapper until `freelocale` in Drop.
        let locale =
            unsafe { libc::newlocale(libc::LC_ALL_MASK, name.as_ptr(), std::ptr::null_mut()) };
        if locale.is_null() {
            Err(Error::new(
                ErrorKind::Protocol,
                format!("could not load locale {name:?} for preview"),
            ))
        } else {
            Ok(Self(locale))
        }
    }

    fn langinfo(&self, item: libc::nl_item) -> Result<String, Error> {
        // SAFETY: the locale handle remains valid for this call and glibc
        // returns a NUL-terminated string owned by the locale object.
        let value = unsafe { libc::nl_langinfo_l(item, self.0) };
        if value.is_null() {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the locale did not provide format information",
            ));
        }
        // SAFETY: `nl_langinfo_l` returned a non-null NUL-terminated string,
        // and it is copied before the locale handle can be freed.
        let value = unsafe { std::ffi::CStr::from_ptr(value) };
        bounded_preview(value.to_string_lossy().into_owned())
    }

    fn date_time(&self) -> Result<String, Error> {
        let format = std::ffi::CString::new("%c").expect("static format has no NUL");
        let mut output = [0_u8; 256];
        // January 15, 2024 at 13:45:00, a Monday. This deterministic sample
        // makes locale previews comparable without reading or changing time.
        // SAFETY: a zeroed `libc::tm` is valid, and every field consumed by
        // this fixed formatting call is populated immediately below.
        let mut time: libc::tm = unsafe { std::mem::zeroed() };
        time.tm_sec = 0;
        time.tm_min = 45;
        time.tm_hour = 13;
        time.tm_mday = 15;
        time.tm_mon = 0;
        time.tm_year = 124;
        time.tm_wday = 1;
        time.tm_yday = 14;
        time.tm_isdst = -1;
        // SAFETY: `output` is writable for its full declared length, `format`
        // is NUL-terminated, `time` is initialized, and the locale is live.
        let written = unsafe {
            libc::strftime_l(
                output.as_mut_ptr().cast(),
                output.len(),
                format.as_ptr(),
                &time,
                self.0,
            )
        };
        if written == 0 {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the locale date and time preview was too long",
            ));
        }
        String::from_utf8(output[..written].to_vec()).map_err(|_| {
            Error::new(
                ErrorKind::Protocol,
                "the locale date and time preview is not UTF-8",
            )
        })
    }

    fn currency(&self) -> Result<String, Error> {
        let format = std::ffi::CString::new("%n").expect("static format has no NUL");
        let mut output = [0_u8; 256];
        // SAFETY: `output` is writable for its full length, the format is a
        // NUL-terminated fixed `%n`, the variadic argument has the required
        // `double` type, and the locale handle remains live for the call.
        let written = unsafe {
            strfmon_l(
                output.as_mut_ptr().cast(),
                output.len(),
                self.0,
                format.as_ptr(),
                1234.56_f64,
            )
        };
        if written < 0 {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the locale could not format a currency preview",
            ));
        }
        String::from_utf8(output[..written as usize].to_vec()).map_err(|_| {
            Error::new(
                ErrorKind::Protocol,
                "the locale currency preview is not UTF-8",
            )
        })
    }
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn strfmon_l(
        output: *mut libc::c_char,
        max: libc::size_t,
        locale: libc::locale_t,
        format: *const libc::c_char,
        ...
    ) -> libc::ssize_t;
}

#[cfg(target_os = "linux")]
impl Drop for NativeLocale {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the non-null locale handle exactly once.
        unsafe { libc::freelocale(self.0) };
    }
}

#[cfg(any(target_os = "linux", test))]
fn grouped_number(thousands: &str, decimal: &str) -> String {
    format!("1{thousands}234{decimal}56")
}

#[cfg(target_os = "linux")]
fn bounded_preview(value: String) -> Result<String, Error> {
    if value.len() > 255 || value.chars().any(|character| character.is_control()) {
        Err(Error::new(
            ErrorKind::Protocol,
            "the locale returned invalid preview text",
        ))
    } else {
        Ok(value)
    }
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

    #[test]
    fn number_example_follows_locale_separators() {
        let number = grouped_number(" ", ",");
        assert_eq!(number, "1 234,56");
    }
}
