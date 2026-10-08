//! The Lulo apps that run on Windows (ADR 0023), by the executable each
//! ships as beside `lulo-shell.exe`. Plain data, so the table is tested on
//! every platform; the Windows shell, its compositor backend and its app
//! catalogue all read it.

use crate::identity;

/// A Lulo app that runs on Windows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowsApp {
    /// The app's identity, as on Lulo OS (`rmac_apps::identity`).
    pub app_id: &'static str,
    pub name: &'static str,
    /// The executable's file name, beside `lulo-shell.exe`.
    pub exe: &'static str,
}

/// Every Lulo app that builds for Windows, in the Dock's order: Files first,
/// where the Mac's Dock has the Finder.
pub const WINDOWS_APPS: [WindowsApp; 9] = [
    WindowsApp {
        app_id: identity::FILES,
        name: "Files",
        exe: "rmac-files.exe",
    },
    WindowsApp {
        app_id: identity::NOTES,
        name: "Notes",
        exe: "rmac-notes.exe",
    },
    WindowsApp {
        app_id: identity::TEXT_EDITOR,
        name: "Text Editor",
        exe: "rmac-text-editor.exe",
    },
    WindowsApp {
        app_id: identity::TERMINAL,
        name: "Terminal",
        exe: "rmac-terminal.exe",
    },
    WindowsApp {
        app_id: identity::SYSTEM_SETTINGS,
        name: "Settings",
        exe: "rmac-system-settings.exe",
    },
    WindowsApp {
        app_id: identity::CALCULATOR,
        name: "Calculator",
        exe: "rmac-calculator.exe",
    },
    WindowsApp {
        app_id: identity::CLOCK,
        name: "Clock",
        exe: "rmac-clock.exe",
    },
    WindowsApp {
        app_id: identity::WEATHER,
        name: "Weather",
        exe: "rmac-weather.exe",
    },
    WindowsApp {
        app_id: identity::PREVIEW,
        name: "Preview",
        exe: "rmac-preview.exe",
    },
];

/// Where the Windows shell keeps the Lulo apps' desktop entries and icons,
/// laid out as Lulo OS's `/usr/share` (`applications/`,
/// `icons/hicolor/scalable/apps/`, `rmac/dock/icons/`), so the Dock,
/// Spotlight and the app catalogue read them exactly as on Lulo OS: the XDG
/// data home every Lulo app on Windows has (`XDG_DATA_HOME`, which
/// `rmac_ui::application` sets to `%APPDATA%\Lulo\Data`). `lulo-shell`
/// writes it as it starts.
pub fn data_home() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("APPDATA")
                .map(std::path::PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join("Lulo").join("Data"))
        })
}

/// The program a desktop entry starts (its `Exec`'s first word), for
/// opening dropped files with that app.
pub fn desktop_entry_program(entry: &std::path::Path) -> Option<std::path::PathBuf> {
    let contents = std::fs::read_to_string(entry).ok()?;
    let values = crate::desktop_group_named(&contents, "Desktop Entry");
    let exec = values.get("Exec")?;
    let exec = exec.trim();
    let program = if let Some(quoted) = exec.strip_prefix('"') {
        quoted.split('"').next()?
    } else {
        exec.split_whitespace().next()?
    };
    (!program.is_empty()).then(|| std::path::PathBuf::from(program))
}

/// The Lulo app whose executable is `file_name` (any case).
pub fn app_for_exe(file_name: &str) -> Option<&'static WindowsApp> {
    WINDOWS_APPS
        .iter()
        .find(|app| app.exe.eq_ignore_ascii_case(file_name))
}

/// The Lulo app with identity `app_id` (a `.desktop` suffix is ignored).
pub fn app(app_id: &str) -> Option<&'static WindowsApp> {
    let app_id = app_id.trim_end_matches(".desktop");
    WINDOWS_APPS.iter().find(|app| app.app_id == app_id)
}

/// The Lulo app whose Windows AppUserModelID is `aumid` (`Lulo.Files` for
/// `org.rmac.Files`, as `rmac_ui` sets it and the installer's shortcuts
/// carry it; any case, as Windows compares them).
pub fn app_for_aumid(aumid: &str) -> Option<&'static WindowsApp> {
    let aumid = aumid.trim();
    let name = aumid
        .get(..5)
        .filter(|prefix| prefix.eq_ignore_ascii_case("Lulo."))
        .and_then(|_| aumid.get(5..))
        .filter(|name| !name.is_empty())?;
    WINDOWS_APPS.iter().find(|app| {
        app.app_id
            .strip_prefix("org.rmac.")
            .is_some_and(|short| short.eq_ignore_ascii_case(name))
    })
}

/// The executable's file name from a full path, lower-cased: the identity a
/// Windows app (one that is not a Lulo app) has in the compositor snapshot,
/// the Dock and the app catalogue.
pub fn exe_key(path: &str) -> String {
    path.rsplit(['\\', '/'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase()
}

/// The app id a window has in the shell: a Lulo app's identity (by its
/// executable, or its window's AppUserModelID when the executable was
/// renamed), a Store app's AppUserModelID (they all run in
/// `ApplicationFrameHost.exe`), else the executable's file name.
pub fn window_app_id(exe_path: &str, aumid: Option<&str>) -> String {
    let key = exe_key(exe_path);
    if let Some(app) = app_for_exe(&key).or_else(|| aumid.and_then(app_for_aumid)) {
        return app.app_id.to_owned();
    }
    match aumid.map(str::trim).filter(|aumid| !aumid.is_empty()) {
        Some(aumid) if key == "applicationframehost.exe" || key.is_empty() => aumid.to_owned(),
        _ => key,
    }
}

static DISPLAY_NAMES: std::sync::RwLock<Option<std::collections::HashMap<String, String>>> =
    std::sync::RwLock::new(None);

/// Remember what an app id without a desktop entry is called (a Windows
/// app's executable description, a Store app's title), so the menu bar and
/// the Dock name it as Windows does. Lulo OS registers nothing here.
pub fn register_display_name(app_id: &str, name: &str) {
    let name = name.trim();
    if app_id.is_empty() || name.is_empty() {
        return;
    }
    if let Ok(mut names) = DISPLAY_NAMES.write() {
        names
            .get_or_insert_with(std::collections::HashMap::new)
            .insert(app_id.to_owned(), name.to_owned());
    }
}

/// The name [`register_display_name`] recorded for `app_id`.
pub fn display_name_for(app_id: &str) -> Option<String> {
    DISPLAY_NAMES
        .read()
        .ok()?
        .as_ref()?
        .get(app_id)
        .cloned()
}

/// How a Windows app is named when its executable carries no description:
/// its file stem, first letter capital.
pub fn name_from_exe(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = file
        .rsplit_once('.')
        .map_or(file, |(stem, _)| stem)
        .to_owned();
    let mut chars = stem.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => stem,
    }
}

/// The name a Windows app shows in the menu bar and the Dock: Windows' own
/// shell processes read wrongly by their description, so they get the
/// Mac-like name; others their executable's description, else their file
/// stem.
pub fn display_name(exe_key: &str, description: Option<&str>) -> String {
    match exe_key {
        "explorer.exe" => "File Explorer".to_owned(),
        _ => description
            .map(str::trim)
            .filter(|description| !description.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| name_from_exe(exe_key)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lulo_apps_are_found_by_executable_in_any_case() {
        assert_eq!(
            app_for_exe("RMAC-Calculator.EXE").map(|app| app.name),
            Some("Calculator")
        );
        assert!(app_for_exe("calc.exe").is_none());
        assert_eq!(app("org.rmac.Notes").map(|app| app.exe), Some("rmac-notes.exe"));
        assert_eq!(
            app("org.rmac.Notes.desktop").map(|app| app.exe),
            Some("rmac-notes.exe")
        );
        for app in WINDOWS_APPS {
            assert!(app.exe.starts_with("rmac-") && app.exe.ends_with(".exe"));
            assert!(identity::window_title(app.app_id).is_some());
        }
        assert_eq!(WINDOWS_APPS[0].app_id, identity::FILES);
    }

    #[test]
    fn lulo_apps_are_found_by_app_user_model_id() {
        assert_eq!(
            app_for_aumid("Lulo.Files").map(|app| app.exe),
            Some("rmac-files.exe")
        );
        assert_eq!(
            app_for_aumid("lulo.texteditor").map(|app| app.name),
            Some("Text Editor")
        );
        assert!(app_for_aumid("Microsoft.WindowsCalculator").is_none());
        assert!(app_for_aumid("Lulo.").is_none());
    }

    #[test]
    fn windows_get_the_shells_app_ids() {
        assert_eq!(
            window_app_id(r"C:\Program Files\Lulo\rmac-files.exe", None),
            identity::FILES
        );
        // Files renamed is still Files, by its AppUserModelID; never Preview.
        assert_eq!(
            window_app_id(r"D:\dev\files-dev.exe", Some("Lulo.Files")),
            identity::FILES
        );
        assert_eq!(
            window_app_id(r"C:\Windows\System32\NOTEPAD.EXE", None),
            "notepad.exe"
        );
        assert_eq!(
            window_app_id(
                r"C:\Windows\System32\ApplicationFrameHost.exe",
                Some("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App")
            ),
            "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"
        );
        // A desktop app's own AppUserModelID does not replace its executable.
        assert_eq!(
            window_app_id(r"C:\Apps\code.exe", Some("Microsoft.VisualStudioCode")),
            "code.exe"
        );
    }

    #[test]
    fn executables_name_windows_apps() {
        assert_eq!(exe_key(r"C:\Windows\System32\NOTEPAD.EXE"), "notepad.exe");
        assert_eq!(name_from_exe(r"C:\Tools\paint.net.exe"), "Paint.net");
        assert_eq!(
            display_name("explorer.exe", Some("Windows Explorer")),
            "File Explorer"
        );
        assert_eq!(display_name("notepad.exe", Some(" Notepad ")), "Notepad");
        assert_eq!(display_name("foo.exe", Some("")), "Foo");
    }
}
