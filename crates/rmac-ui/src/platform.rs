//! The few process-wide facts that differ between the platforms rmac apps run
//! on (ADR 0023).
//!
//! Every rmac crate finds its files through the XDG base-directory variables
//! (`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME`)
//! and `HOME`. Linux and macOS set `HOME`, and the XDG defaults hang off it.
//! Windows sets neither, so [`prepare_environment`] points them at the
//! per-user Windows folders before any thread starts:
//!
//! | Variable          | Windows folder                    |
//! |-------------------|-----------------------------------|
//! | `HOME`            | `%USERPROFILE%`                   |
//! | `XDG_CONFIG_HOME` | `%APPDATA%\Lulo\Config`           |
//! | `XDG_DATA_HOME`   | `%APPDATA%\Lulo\Data`             |
//! | `XDG_STATE_HOME`  | `%LOCALAPPDATA%\Lulo\State`       |
//! | `XDG_CACHE_HOME`  | `%LOCALAPPDATA%\Lulo\Cache`       |
//!
//! Roaming `%APPDATA%` holds what a user expects to follow them (settings and
//! documents such as notes); `%LOCALAPPDATA%` holds machine-local state and
//! caches. A variable that is already set is left alone, so tests and the
//! launch smoke check can point an app at a private profile.

/// Fill in the base-directory variables this platform does not set. Called by
/// [`crate::application`] before GPUI starts, while the process is still
/// single-threaded.
pub(crate) fn prepare_environment() {
    #[cfg(windows)]
    for (name, value) in windows_defaults(&|name| std::env::var_os(name)) {
        std::env::set_var(name, value);
    }
}

/// The variables to set, given a way to read the current environment.
#[cfg(any(windows, test))]
fn windows_defaults(
    read: &dyn Fn(&str) -> Option<std::ffi::OsString>,
) -> Vec<(&'static str, std::path::PathBuf)> {
    use std::path::PathBuf;

    let folder = |variable: &str| {
        read(variable)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let roaming = folder("APPDATA").map(|path| path.join("Lulo"));
    let local = folder("LOCALAPPDATA").map(|path| path.join("Lulo"));
    let wanted = [
        ("HOME", folder("USERPROFILE")),
        (
            "XDG_CONFIG_HOME",
            roaming.as_ref().map(|path| path.join("Config")),
        ),
        (
            "XDG_DATA_HOME",
            roaming.as_ref().map(|path| path.join("Data")),
        ),
        (
            "XDG_STATE_HOME",
            local.as_ref().map(|path| path.join("State")),
        ),
        (
            "XDG_CACHE_HOME",
            local.as_ref().map(|path| path.join("Cache")),
        ),
    ];
    wanted
        .into_iter()
        .filter(|(name, _)| read(name).is_none_or(|value| value.is_empty()))
        .filter_map(|(name, value)| Some((name, value?)))
        .collect()
}

/// The first code of the process that is Lulo's: the C runtime runs it
/// with its other initializers, before `main`. With the start-up trace on,
/// time before it is the Windows loader's (mapping the exe and its DLLs),
/// time after it and before an app's first mark is the app's own.
#[cfg(windows)]
#[used]
#[link_section = ".CRT$XCU"]
static CRT_INIT_MARK: extern "C" fn() = crt_init_mark;

#[cfg(windows)]
extern "C" fn crt_init_mark() {
    startup_mark("crt_init");
}

/// See [`crate::startup_mark`].
#[cfg(windows)]
pub(crate) fn startup_mark(phase: &str) {
    use std::io::Write as _;
    use std::sync::OnceLock;
    use windows::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetProcessTimes},
    };

    static ENABLED: OnceLock<bool> = OnceLock::new();
    if !*ENABLED
        .get_or_init(|| std::env::var("RMAC_GPUI_STARTUP_TRACE").is_ok_and(|value| value == "1"))
    {
        return;
    }
    let [mut creation, mut exit, mut kernel, mut user] = [FILETIME::default(); 4];
    // SAFETY: the pseudo handle is always valid and the out pointers point
    // at live locals.
    if unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    }
    .is_err()
    {
        return;
    }
    let created = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
    // FILETIME counts 100 ns from 1601; the Unix epoch is 11 644 473 600 s on.
    let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return;
    };
    let now = (now.as_nanos() / 100) as u64 + 11_644_473_600 * 10_000_000;
    let ms = now.saturating_sub(created) as f64 / 10_000.0;
    // One write per line, as gpui_windows' own trace does.
    let line = format!("gpui_windows startup: {phase} at {ms:.1} ms\n");
    let _ = std::io::stderr().lock().write_all(line.as_bytes());
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::windows_defaults;

    fn absolute(path: &str) -> PathBuf {
        // An absolute path on every host the tests run on.
        std::env::temp_dir().join(path)
    }

    #[test]
    fn windows_folders_fill_only_the_unset_variables() {
        let environment = HashMap::from([
            ("USERPROFILE", absolute("profile").into_os_string()),
            ("APPDATA", absolute("roaming").into_os_string()),
            ("LOCALAPPDATA", absolute("local").into_os_string()),
            ("XDG_CACHE_HOME", OsString::from("/already/set")),
        ]);
        let read = |name: &str| environment.get(name).cloned();
        let defaults = windows_defaults(&read);
        assert_eq!(
            defaults,
            vec![
                ("HOME", absolute("profile")),
                (
                    "XDG_CONFIG_HOME",
                    absolute("roaming").join("Lulo").join("Config")
                ),
                (
                    "XDG_DATA_HOME",
                    absolute("roaming").join("Lulo").join("Data")
                ),
                (
                    "XDG_STATE_HOME",
                    absolute("local").join("Lulo").join("State")
                ),
            ]
        );
    }

    #[test]
    fn relative_or_missing_windows_folders_are_ignored() {
        let environment = HashMap::from([("APPDATA", OsString::from("relative"))]);
        let read = |name: &str| environment.get(name).cloned();
        assert!(windows_defaults(&read).is_empty());
    }
}
