//! The apps the Lulo layer knows by name: the Lulo apps that run on
//! Windows, shipped beside `lulo-shell.exe`, and the Windows apps it pins.

/// A Lulo app that runs on Windows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LuloApp {
    /// The app's identity, as on Lulo OS (`rmac_apps::identity`).
    pub app_id: &'static str,
    pub name: &'static str,
    /// The executable's file name, beside `lulo-shell.exe`.
    pub exe: &'static str,
    /// The app's icon in the shell's assets.
    pub icon: &'static str,
}

/// The Lulo apps that build for Windows (ADR 0023 phases 2 and 4), in the
/// Dock's order: Files first, where the Mac's Dock has the Finder.
pub const LULO_APPS: [LuloApp; 8] = [
    LuloApp {
        app_id: "org.rmac.Files",
        name: "Files",
        exe: "rmac-files.exe",
        icon: "apps/org.rmac.Files.svg",
    },
    LuloApp {
        app_id: "org.rmac.Notes",
        name: "Notes",
        exe: "rmac-notes.exe",
        icon: "apps/org.rmac.Notes.svg",
    },
    LuloApp {
        app_id: "org.rmac.Calculator",
        name: "Calculator",
        exe: "rmac-calculator.exe",
        icon: "apps/org.rmac.Calculator.svg",
    },
    LuloApp {
        app_id: "org.rmac.Clock",
        name: "Clock",
        exe: "rmac-clock.exe",
        icon: "apps/org.rmac.Clock.svg",
    },
    LuloApp {
        app_id: "org.rmac.Weather",
        name: "Weather",
        exe: "rmac-weather.exe",
        icon: "apps/org.rmac.Weather.svg",
    },
    LuloApp {
        app_id: "org.rmac.Preview",
        name: "Preview",
        exe: "rmac-preview.exe",
        icon: "apps/org.rmac.Preview.svg",
    },
    LuloApp {
        app_id: "org.rmac.TextEditor",
        name: "Text Editor",
        exe: "rmac-text-editor.exe",
        icon: "apps/org.rmac.TextEditor.svg",
    },
    LuloApp {
        app_id: "org.rmac.Terminal",
        name: "Terminal",
        exe: "rmac-terminal.exe",
        icon: "apps/org.rmac.Terminal.svg",
    },
];

/// The Lulo app whose executable is `file_name` (any case).
pub fn lulo_app_for_exe(file_name: &str) -> Option<&'static LuloApp> {
    LULO_APPS
        .iter()
        .find(|app| app.exe.eq_ignore_ascii_case(file_name))
}

pub fn lulo_app(app_id: &str) -> Option<&'static LuloApp> {
    LULO_APPS.iter().find(|app| app.app_id == app_id)
}

/// The Lulo app whose Windows AppUserModelID is `aumid` (`Lulo.Files` for
/// `org.rmac.Files`, as `rmac_ui` sets it and the installer's shortcuts
/// carry it; any case, as Windows compares them).
pub fn lulo_app_for_aumid(aumid: &str) -> Option<&'static LuloApp> {
    let aumid = aumid.trim();
    let name = aumid
        .get(..5)
        .filter(|prefix| prefix.eq_ignore_ascii_case("Lulo."))
        .and_then(|_| aumid.get(5..))
        .filter(|name| !name.is_empty())?;
    LULO_APPS.iter().find(|app| {
        app.app_id
            .strip_prefix("org.rmac.")
            .is_some_and(|short| short.eq_ignore_ascii_case(name))
    })
}

/// The Lulo app a running process is, by what is known of it: its
/// executable's file name, the app id it gave the menu bar over the menu
/// pipe, or its window's AppUserModelID.
pub fn identify(
    exe_key: &str,
    pipe_app_id: Option<&str>,
    aumid: Option<&str>,
) -> Option<&'static LuloApp> {
    lulo_app_for_exe(exe_key)
        .or_else(|| pipe_app_id.and_then(lulo_app))
        .or_else(|| aumid.and_then(lulo_app_for_aumid))
}

/// The executable's file name from a full path, lower-cased: the key the
/// Dock groups windows by.
pub fn exe_key(path: &str) -> String {
    path.rsplit(['\\', '/'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase()
}

/// How a Windows app is named in the menu bar and the Dock when its
/// executable carries no description: its file stem, first letter capital.
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

/// Names Windows gives its own shell processes that read wrongly as app
/// names; the Mac-like name is used instead.
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
            lulo_app_for_exe("RMAC-Calculator.EXE").map(|app| app.name),
            Some("Calculator")
        );
        assert!(lulo_app_for_exe("calc.exe").is_none());
        assert_eq!(
            lulo_app_for_exe("rmac-files.exe").map(|app| app.icon),
            Some("apps/org.rmac.Files.svg")
        );
        assert_eq!(
            lulo_app("org.rmac.Notes").map(|app| app.exe),
            Some("rmac-notes.exe")
        );
        for app in LULO_APPS {
            assert!(app.icon.ends_with(".svg"));
            assert!(app.exe.starts_with("rmac-"));
        }
    }

    #[test]
    fn lulo_apps_are_found_by_app_id_and_app_user_model_id() {
        assert_eq!(
            lulo_app_for_aumid("Lulo.Files").map(|app| app.exe),
            Some("rmac-files.exe")
        );
        assert_eq!(
            lulo_app_for_aumid("lulo.texteditor").map(|app| app.name),
            Some("Text Editor")
        );
        assert!(lulo_app_for_aumid("Microsoft.WindowsCalculator").is_none());
        assert!(lulo_app_for_aumid("Lulo.").is_none());
        // Files renamed or run from elsewhere is still Files, by the id it
        // gave the bar or by its AppUserModelID; never Preview.
        assert_eq!(
            identify("files-dev.exe", Some("org.rmac.Files"), None).map(|app| app.name),
            Some("Files")
        );
        assert_eq!(
            identify("files-dev.exe", None, Some("Lulo.Files")).map(|app| app.name),
            Some("Files")
        );
        assert_eq!(
            identify("rmac-files.exe", None, None).map(|app| app.name),
            Some("Files")
        );
        assert!(identify("notepad.exe", None, None).is_none());
    }

    #[test]
    fn executables_key_and_name_windows_apps() {
        assert_eq!(exe_key(r"C:\Windows\System32\NOTEPAD.EXE"), "notepad.exe");
        assert_eq!(exe_key("calc.exe"), "calc.exe");
        assert_eq!(name_from_exe(r"C:\Tools\paint.net.exe"), "Paint.net");
        assert_eq!(
            display_name("explorer.exe", Some("Windows Explorer")),
            "File Explorer"
        );
        assert_eq!(display_name("notepad.exe", Some(" Notepad ")), "Notepad");
        assert_eq!(display_name("foo.exe", Some("")), "Foo");
    }
}
