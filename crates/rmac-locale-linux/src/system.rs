#[cfg(target_os = "linux")]
use rmac_locale::FormatPreview;
use rmac_locale::{Error, ErrorKind, Snapshot};

#[cfg(target_os = "linux")]
const MAX_LOCALE_INVENTORY_BYTES: usize = 1024 * 1024;
#[cfg(target_os = "linux")]
const MAX_XKB_INVENTORY_BYTES: usize = 256 * 1024;

#[cfg(target_os = "linux")]
pub(crate) fn system_snapshot() -> Result<Snapshot, Error> {
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
pub(crate) fn system_hour_cycle() -> Result<rmac_locale::HourCycle, Error> {
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
pub(crate) fn hour_cycle_from_time_format(format: &str) -> Option<rmac_locale::HourCycle> {
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
pub(crate) fn grouped_number(thousands: &str, decimal: &str) -> String {
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
pub(crate) fn system_snapshot() -> Result<Snapshot, Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "language and region settings are available in the supported Linux session",
    ))
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn system_hour_cycle() -> Result<rmac_locale::HourCycle, Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "locale hour-cycle settings are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn system_set_locale(assignments: &[String]) -> Result<(), Error> {
    let connection = system_connection()?;
    let proxy = locale_proxy(&connection)?;
    proxy
        .call::<_, _, ()>("SetLocale", &(assignments, true))
        .map_err(mutation_error)
}

#[cfg(target_os = "linux")]
pub(crate) fn system_set_x11_keyboard(keyboard: &rmac_locale::X11Keyboard) -> Result<(), Error> {
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
pub(crate) fn system_set_x11_keyboard(_keyboard: &rmac_locale::X11Keyboard) -> Result<(), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "keyboard layout changes are available in the supported Linux session",
    ))
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn system_set_locale(_assignments: &[String]) -> Result<(), Error> {
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
    rmac_dbus::system_blocking().map_err(|_| {
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
