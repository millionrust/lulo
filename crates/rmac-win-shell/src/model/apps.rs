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

/// The Lulo apps that build for Windows (ADR 0023 phase 2), in the Dock's
/// order.
pub const LULO_APPS: [LuloApp; 7] = [
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
            lulo_app("org.rmac.Notes").map(|app| app.exe),
            Some("rmac-notes.exe")
        );
        for app in LULO_APPS {
            assert!(app.icon.ends_with(".svg"));
            assert!(app.exe.starts_with("rmac-"));
        }
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
