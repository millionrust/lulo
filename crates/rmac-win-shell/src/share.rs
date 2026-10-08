//! The Lulo apps' desktop entries and artwork, laid out on Windows as Lulo
//! OS lays them out under `/usr/share` (ADR 0023, "Phase 3 revised: shared
//! shell views"), in `rmac_apps::windows_apps::data_home()`
//! (`%APPDATA%\Lulo\Data`, the XDG data home). The Dock, the desktop and the app
//! catalogue then read them exactly as on Lulo OS: the same names, the same
//! icons, the same categories.
//!
//! The files are the ones Lulo OS ships (`packaging/rmac-apps`,
//! `crates/rmac-dock/assets/icons`), compiled in. A desktop entry is written
//! only for an app whose executable is beside `lulo-shell.exe`, and its
//! `Exec` names that executable. A file is written only when it differs, so
//! a normal start writes nothing.

use std::path::{Path, PathBuf};

macro_rules! files {
    ($($name:literal => $path:literal),* $(,)?) => {
        &[$(($name, include_bytes!($path) as &[u8])),*]
    };
}

/// Desktop entries by the executable they start.
const ENTRIES: &[(&str, &[u8])] = files! {
    "org.rmac.Files" => "../../../packaging/rmac-apps/applications/org.rmac.Files.desktop",
    "org.rmac.Notes" => "../../../packaging/rmac-apps/applications/org.rmac.Notes.desktop",
    "org.rmac.TextEditor" => "../../../packaging/rmac-apps/applications/org.rmac.TextEditor.desktop",
    "org.rmac.Terminal" => "../../../packaging/rmac-apps/applications/org.rmac.Terminal.desktop",
    "org.rmac.SystemSettings" => "../../../packaging/rmac-apps/applications/org.rmac.SystemSettings.desktop",
    "org.rmac.Calculator" => "../../../packaging/rmac-apps/applications/org.rmac.Calculator.desktop",
    "org.rmac.Clock" => "../../../packaging/rmac-apps/applications/org.rmac.Clock.desktop",
    "org.rmac.Weather" => "../../../packaging/rmac-apps/applications/org.rmac.Weather.desktop",
    "org.rmac.Preview" => "../../../packaging/rmac-apps/applications/org.rmac.Preview.desktop",
};

/// App icons, as `icons/hicolor/scalable/apps/<name>.svg`.
const APP_ICONS: &[(&str, &[u8])] = files! {
    "org.rmac.Files.svg" => "../../../packaging/rmac-apps/icons/org.rmac.Files.svg",
    "org.rmac.Notes.svg" => "../../../packaging/rmac-apps/icons/org.rmac.Notes.svg",
    "org.rmac.TextEditor.svg" => "../../../packaging/rmac-apps/icons/org.rmac.TextEditor.svg",
    "org.rmac.Terminal.svg" => "../../../packaging/rmac-apps/icons/org.rmac.Terminal.svg",
    "org.rmac.SystemSettings.svg" => "../../../packaging/rmac-apps/icons/org.rmac.SystemSettings.svg",
    "org.rmac.Calculator.svg" => "../../../packaging/rmac-apps/icons/org.rmac.Calculator.svg",
    "org.rmac.Clock.svg" => "../../../packaging/rmac-apps/icons/org.rmac.Clock.svg",
    "org.rmac.Weather.svg" => "../../../packaging/rmac-apps/icons/org.rmac.Weather.svg",
    "org.rmac.Preview.svg" => "../../../packaging/rmac-apps/icons/org.rmac.Preview.svg",
    "org.rmac.AppDrawer.svg" => "../../../packaging/rmac-apps/icons/org.rmac.AppDrawer.svg",
    "org.rmac.Mail.svg" => "../../../packaging/rmac-apps/icons/org.rmac.Mail.svg",
    "org.rmac.Calendar.svg" => "../../../packaging/rmac-apps/icons/org.rmac.Calendar.svg",
    "org.rmac.SystemMonitor.svg" => "../../../packaging/rmac-apps/icons/org.rmac.SystemMonitor.svg",
    "org.rmac.Player.svg" => "../../../packaging/rmac-apps/icons/org.rmac.Player.svg",
};

/// The Dock's own artwork, as `rmac/dock/icons/<name>`.
const DOCK_ICONS: &[(&str, &[u8])] = files! {
    "application.svg" => "../../rmac-dock/assets/icons/application.svg",
    "downloads.svg" => "../../rmac-dock/assets/icons/downloads.svg",
    "files.svg" => "../../rmac-dock/assets/icons/files.svg",
    "folder.svg" => "../../rmac-dock/assets/icons/folder.svg",
    "more.svg" => "../../rmac-dock/assets/icons/more.svg",
    "stack-item-document.svg" => "../../rmac-dock/assets/icons/stack-item-document.svg",
    "trash-empty.svg" => "../../rmac-dock/assets/icons/trash-empty.svg",
    "trash-full.svg" => "../../rmac-dock/assets/icons/trash-full.svg",
};

/// The hicolor theme's index, so named icons resolve as on Lulo OS.
const HICOLOR_INDEX: &str = "[Icon Theme]\nName=Hicolor\nComment=Fallback icon theme\nDirectories=scalable/apps\n\n[scalable/apps]\nSize=64\nMinSize=8\nMaxSize=1024\nContext=Applications\nType=Scalable\n";

fn write_if_changed(path: &Path, contents: &[u8]) {
    if std::fs::read(path).is_ok_and(|current| current == contents) {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = std::fs::write(path, contents) {
        eprintln!("lulo-shell: could not write {}: {error}", path.display());
    }
}

/// `entry` with every `/usr/bin/<program>` in its `Exec` and `TryExec`
/// lines replaced by `<program>.exe` in `install_dir`, quoted, with forward
/// slashes (desktop-entry quoting treats a backslash as an escape).
pub fn windows_entry(entry: &str, install_dir: &Path) -> String {
    let directory = install_dir.to_string_lossy().replace('\\', "/");
    entry
        .lines()
        .map(|line| {
            let Some((key, value)) = line.split_once('=') else {
                return line.to_owned();
            };
            if key != "Exec" && key != "TryExec" {
                return line.to_owned();
            }
            let mut words = value.splitn(2, ' ');
            let program = words.next().unwrap_or_default();
            let rest = words.next();
            let Some(name) = program.strip_prefix("/usr/bin/") else {
                return line.to_owned();
            };
            let exe = format!("{directory}/{name}.exe");
            let exe = if key == "Exec" {
                format!("\"{exe}\"")
            } else {
                exe
            };
            match rest {
                Some(rest) => format!("{key}={exe} {rest}"),
                None => format!("{key}={exe}"),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// Write the share directory for the Lulo apps installed in `install_dir`.
pub fn install(install_dir: &Path) {
    let Some(share) = rmac_apps::windows_apps::data_home() else {
        return;
    };
    // The shared-view checks' fixed scene draws the Dock from every entry,
    // whichever apps the check built.
    let scene = std::env::var_os("RMAC_SHELL_SCENE").is_some_and(|value| value == "1");
    install_into(&share, install_dir, scene);
}

pub fn install_into(share: &Path, install_dir: &Path, scene: bool) {
    let applications = share.join("applications");
    for (id, contents) in ENTRIES {
        let path = applications.join(format!("{id}.desktop"));
        let Some(app) = rmac_apps::windows_apps::app(id) else {
            continue;
        };
        if !scene && !install_dir.join(app.exe).is_file() {
            // Not installed on this PC: no entry, so the Dock and Spotlight
            // never offer an app that cannot start.
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let mut entry = windows_entry(&String::from_utf8_lossy(contents), install_dir);
        if scene {
            entry = entry
                .lines()
                .filter(|line| !line.starts_with("TryExec="))
                .map(|line| format!("{line}\n"))
                .collect();
        }
        write_if_changed(&path, entry.as_bytes());
    }
    let icons = share.join("icons").join("hicolor");
    write_if_changed(&icons.join("index.theme"), HICOLOR_INDEX.as_bytes());
    for (name, contents) in APP_ICONS {
        write_if_changed(&icons.join("scalable").join("apps").join(name), contents);
    }
    let dock = share.join("rmac").join("dock").join("icons");
    for (name, contents) in DOCK_ICONS {
        write_if_changed(&dock.join(name), contents);
    }
}

/// The folder `lulo-shell.exe` runs from, where the Lulo apps are.
pub fn install_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_start_the_executable_beside_the_shell() {
        let entry = "[Desktop Entry]\nTryExec=/usr/bin/rmac-files\nExec=/usr/bin/rmac-files %U\nIcon=org.rmac.Files\n";
        let windows = windows_entry(entry, Path::new(r"C:\Program Files\Lulo"));
        assert!(windows.contains("TryExec=C:/Program Files/Lulo/rmac-files.exe\n"));
        assert!(windows.contains("Exec=\"C:/Program Files/Lulo/rmac-files.exe\" %U\n"));
        assert!(windows.contains("Icon=org.rmac.Files\n"));
    }

    #[test]
    fn every_windows_app_has_an_entry_and_an_icon() {
        for app in rmac_apps::windows_apps::WINDOWS_APPS {
            assert!(
                ENTRIES.iter().any(|(id, _)| *id == app.app_id),
                "{}",
                app.app_id
            );
            assert!(
                APP_ICONS
                    .iter()
                    .any(|(name, _)| *name == format!("{}.svg", app.app_id)),
                "{}",
                app.app_id
            );
        }
    }

    #[test]
    fn the_catalogue_reads_what_is_installed() {
        let root = std::env::temp_dir().join(format!("lulo-share-{}", std::process::id()));
        let install_dir = root.join("bin");
        std::fs::create_dir_all(&install_dir).unwrap();
        std::fs::write(install_dir.join("rmac-files.exe"), b"").unwrap();
        let share = root.join("share");
        install_into(&share, &install_dir, false);
        assert!(share.join("applications/org.rmac.Files.desktop").is_file());
        assert!(!share.join("applications/org.rmac.Notes.desktop").exists());
        assert!(share
            .join("icons/hicolor/scalable/apps/org.rmac.Files.svg")
            .is_file());
        assert!(share.join("rmac/dock/icons/trash-empty.svg").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
