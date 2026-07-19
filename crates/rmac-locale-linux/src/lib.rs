//! Linux systemd-localed adapter.

use rmac_locale::{Error, ErrorKind, Service, Snapshot};

#[cfg(target_os = "linux")]
use rmac_locale::FormatPreview;

#[cfg(target_os = "linux")]
const MAX_LOCALE_INVENTORY_BYTES: usize = 1024 * 1024;
#[cfg(target_os = "linux")]
const MAX_XKB_INVENTORY_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemService;

impl Service for SystemService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        system_snapshot()
    }

    fn set_locale(&self, assignments: &[String]) -> Result<Snapshot, Error> {
        let current = self.snapshot()?;
        let desired = rmac_locale::normalize_assignments(assignments.to_vec())?;
        for assignment in &desired {
            let unchanged = current.locale.iter().any(|candidate| {
                candidate.key == assignment.key && candidate.value == assignment.value
            });
            if assignment.key != "LANGUAGE" && !unchanged {
                current.validate_installed(&assignment.value)?;
            }
        }
        apply_complete_locale(current, &desired)
    }

    fn set_x11_keyboard(&self, keyboard: &rmac_locale::X11Keyboard) -> Result<Snapshot, Error> {
        let current = self.snapshot()?;
        let validated =
            current.preview_x11_keyboard(&keyboard.layout, &keyboard.variant, &keyboard.options)?;
        if keyboard.model != validated.model {
            return Err(Error::new(
                ErrorKind::InvalidKeyboard,
                "this control preserves the current XKB model",
            ));
        }
        if current.x11_keyboard() == validated {
            return Ok(current);
        }
        system_set_x11_keyboard(&validated)?;
        let after = self.snapshot()?;
        if after.x11_keyboard() != validated {
            return Err(Error::new(
                ErrorKind::Mismatch,
                "localed did not confirm the requested keyboard layout",
            ));
        }
        Ok(after)
    }
}

pub fn snapshot() -> Result<Snapshot, Error> {
    SystemService.snapshot()
}

/// Return the system locale's authoritative hour cycle without enumerating
/// installed locales or keyboard layouts.
pub fn hour_cycle() -> Result<rmac_locale::HourCycle, Error> {
    system_hour_cycle()
}

pub fn set_locale(assignments: &[String]) -> Result<Snapshot, Error> {
    SystemService.set_locale(assignments)
}

pub fn set_x11_keyboard(keyboard: &rmac_locale::X11Keyboard) -> Result<Snapshot, Error> {
    SystemService.set_x11_keyboard(keyboard)
}

pub fn restore_locale(rollback: &rmac_locale::LocaleRollback) -> Result<Snapshot, Error> {
    let current = SystemService.snapshot()?;
    if !rmac_locale::locale_assignments_match(&current.locale, rollback.expected()) {
        return Err(Error::new(
            ErrorKind::Conflict,
            "the system locale changed after rmac applied it; refresh before reverting",
        ));
    }
    apply_complete_locale(current, rollback.previous())
}

pub fn restore_x11_keyboard(rollback: &rmac_locale::KeyboardRollback) -> Result<Snapshot, Error> {
    let current = SystemService.snapshot()?;
    if current.x11_keyboard() != *rollback.expected() {
        return Err(Error::new(
            ErrorKind::Conflict,
            "the keyboard layout changed after rmac applied it; refresh before reverting",
        ));
    }
    let previous = rollback.previous();
    let validated =
        current.preview_x11_keyboard(&previous.layout, &previous.variant, &previous.options)?;
    if validated.model != previous.model {
        return Err(Error::new(
            ErrorKind::Conflict,
            "the authoritative keyboard model changed; the previous layout was not restored",
        ));
    }
    system_set_x11_keyboard(previous)?;
    let after = SystemService.snapshot()?;
    if after.x11_keyboard() != *previous {
        return Err(Error::new(
            ErrorKind::Mismatch,
            "localed did not confirm the previous keyboard layout",
        ));
    }
    Ok(after)
}

fn apply_complete_locale(
    before: Snapshot,
    desired: &[rmac_locale::Assignment],
) -> Result<Snapshot, Error> {
    if rmac_locale::locale_assignments_match(&before.locale, desired) {
        return Ok(before);
    }
    let request = rmac_locale::complete_locale_request(&before.locale, desired);
    system_set_locale(&request)?;
    let mut after = SystemService.snapshot()?;

    // When LANG changes, localed may synthesize LANGUAGE from its fallback
    // table. If that is the sole difference from the desired complete state,
    // confirm the intermediate snapshot is still current, then remove only
    // LANGUAGE in a second request without LANG so fallback is not re-triggered.
    let unexpected = after
        .locale
        .iter()
        .filter(|assignment| {
            !desired
                .iter()
                .any(|candidate| candidate.key == assignment.key)
        })
        .collect::<Vec<_>>();
    let requested_values_match = desired.iter().all(|assignment| {
        after
            .locale
            .iter()
            .any(|candidate| candidate.key == assignment.key && candidate.value == assignment.value)
    });
    if requested_values_match && unexpected.len() == 1 && unexpected[0].key == "LANGUAGE" {
        let confirmed = SystemService.snapshot()?;
        if !rmac_locale::locale_assignments_match(&confirmed.locale, &after.locale) {
            return Err(Error::new(
                ErrorKind::Conflict,
                "the system locale changed while localed was applying the request",
            ));
        }
        system_set_locale(&["LANGUAGE=".into()])?;
        after = SystemService.snapshot()?;
    }
    if !rmac_locale::locale_assignments_match(&after.locale, desired) {
        return Err(Error::new(
            ErrorKind::Mismatch,
            "localed did not confirm the requested language and region state",
        ));
    }
    Ok(after)
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
fn owner_change_reappeared(name: &str, new_owner: &str) -> bool {
    name == "org.freedesktop.locale1" && !new_owner.is_empty()
}

#[cfg(target_os = "linux")]
fn system_snapshot() -> Result<Snapshot, Error> {
    let connection = system_connection()?;
    let proxy = locale_proxy(&connection)?;
    let locale = rmac_locale::normalize_assignments(property(&proxy, "Locale")?)?;
    let (installed_locales, installed_locales_truncated) = installed_locales()?;
    let (installed_x11_layouts, installed_x11_layouts_truncated, x11_layouts_error) =
        match installed_x11_layouts() {
            Ok((layouts, truncated)) => (layouts, truncated, None),
            Err(error) => (Vec::new(), false, Some(error.to_string())),
        };
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
        installed_x11_layouts,
        installed_x11_layouts_truncated,
        x11_layouts_error,
        x11_layout: property(&proxy, "X11Layout")?,
        x11_model: property(&proxy, "X11Model")?,
        x11_variant: property(&proxy, "X11Variant")?,
        x11_options: property(&proxy, "X11Options")?,
        console_keymap: property(&proxy, "VConsoleKeymap")?,
    })
}

#[cfg(target_os = "linux")]
fn system_hour_cycle() -> Result<rmac_locale::HourCycle, Error> {
    let connection = system_connection()?;
    let proxy = locale_proxy(&connection)?;
    let locale = rmac_locale::normalize_assignments(property(&proxy, "Locale")?)?;
    let language = assignment(&locale, "LANG").unwrap_or("C");
    let date_locale = assignment(&locale, "LC_TIME").unwrap_or(language);
    NativeLocale::new(date_locale)?.hour_cycle()
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

    fn hour_cycle(&self) -> Result<rmac_locale::HourCycle, Error> {
        let format = self.langinfo(libc::T_FMT)?;
        hour_cycle_from_time_format(&format).ok_or_else(|| {
            Error::new(
                ErrorKind::Protocol,
                "the locale did not disclose a supported hour cycle",
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

#[cfg(any(target_os = "linux", test))]
fn hour_cycle_from_time_format(format: &str) -> Option<rmac_locale::HourCycle> {
    let mut characters = format.chars();
    while let Some(character) = characters.next() {
        if character != '%' {
            continue;
        }
        let mut directive = characters.next()?;
        if directive == '%' {
            continue;
        }
        if matches!(directive, 'E' | 'O') {
            directive = characters.next()?;
        }
        match directive {
            'I' | 'l' | 'r' => return Some(rmac_locale::HourCycle::TwelveHour),
            'H' | 'k' | 'R' | 'T' => return Some(rmac_locale::HourCycle::TwentyFourHour),
            _ => {}
        }
    }
    None
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

#[cfg(not(target_os = "linux"))]
fn system_hour_cycle() -> Result<rmac_locale::HourCycle, Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "locale hour-cycle settings are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
fn system_set_locale(assignments: &[String]) -> Result<(), Error> {
    let connection = system_connection()?;
    let proxy = locale_proxy(&connection)?;
    proxy
        .call::<_, _, ()>("SetLocale", &(assignments, true))
        .map_err(mutation_error)
}

#[cfg(target_os = "linux")]
fn system_set_x11_keyboard(keyboard: &rmac_locale::X11Keyboard) -> Result<(), Error> {
    rmac_locale::validate_x11_keyboard(&keyboard.layout, &keyboard.variant, &keyboard.options)?;
    let connection = system_connection()?;
    let proxy = locale_proxy(&connection)?;
    proxy
        .call::<_, _, ()>(
            "SetX11Keyboard",
            &(
                keyboard.layout.as_str(),
                keyboard.model.as_str(),
                keyboard.variant.as_str(),
                keyboard.options.as_str(),
                false,
                true,
            ),
        )
        .map_err(mutation_error)
}

#[cfg(not(target_os = "linux"))]
fn system_set_x11_keyboard(_keyboard: &rmac_locale::X11Keyboard) -> Result<(), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "keyboard layout changes are available in the supported Linux session",
    ))
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
    let mut command = std::process::Command::new("locale");
    command.arg("-a");
    let output =
        bounded_command_output(command, MAX_LOCALE_INVENTORY_BYTES, "installed locale list")?;
    Ok(rmac_locale::normalize_installed_locales(
        output.lines().map(str::to_string).collect(),
    ))
}

#[cfg(target_os = "linux")]
fn installed_x11_layouts() -> Result<(Vec<String>, bool), Error> {
    let mut command = std::process::Command::new("localectl");
    command
        .arg("--no-pager")
        .arg("--no-legend")
        .arg("list-x11-keymap-layouts");
    let output = bounded_command_output(
        command,
        MAX_XKB_INVENTORY_BYTES,
        "installed XKB layout list",
    )?;
    Ok(rmac_locale::normalize_installed_x11_layouts(
        output.lines().map(str::to_owned).collect(),
    ))
}

#[cfg(target_os = "linux")]
fn bounded_command_output(
    mut command: std::process::Command,
    max_bytes: usize,
    label: &str,
) -> Result<String, Error> {
    use std::io::Read as _;

    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|_| Error::new(ErrorKind::Unavailable, format!("could not read {label}")))?;
    let mut output = Vec::with_capacity(max_bytes.min(64 * 1024));
    let read = child
        .stdout
        .take()
        .ok_or_else(|| Error::new(ErrorKind::Protocol, format!("could not capture {label}")))?
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut output);
    if read.is_err() || output.len() > max_bytes {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::new(
            ErrorKind::Protocol,
            format!("{label} exceeded its safe output bound"),
        ));
    }
    let status = child.wait().map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            format!("could not finish reading {label}"),
        )
    })?;
    if !status.success() {
        return Err(Error::new(
            ErrorKind::Unavailable,
            format!("could not read {label}"),
        ));
    }
    String::from_utf8(output)
        .map_err(|_| Error::new(ErrorKind::Protocol, format!("{label} is not UTF-8")))
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
        Error::new(
            ErrorKind::Mutation,
            "localed rejected the requested language or keyboard change",
        )
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

    #[test]
    fn hour_cycle_comes_from_the_locale_time_format() {
        assert_eq!(
            hour_cycle_from_time_format("%r"),
            Some(rmac_locale::HourCycle::TwelveHour)
        );
        assert_eq!(
            hour_cycle_from_time_format("%OH:%M:%S"),
            Some(rmac_locale::HourCycle::TwentyFourHour)
        );
        assert_eq!(hour_cycle_from_time_format("%% %Z"), None);
    }
}
