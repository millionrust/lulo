//! Windows' own apps as desktop entries, so Lulo OS's Spotlight finds
//! them and the Dock names a running one (ADR 0023, "Phase 3 revised:
//! shared shell views").
//!
//! The Apps folder (Start menu shortcuts and Store apps) is read by the
//! short-lived `lulo-shell --apps-helper` process (WIN-OS-53), off the UI
//! thread, at start and whenever the Start menu folders change. Each app
//! becomes `<window app id>.desktop` in Lulo's data home: the id the
//! window list gives the app's windows (its executable's file name, or a
//! Store app's AUMID), so the Dock groups the app's windows under it. An
//! entry opens the app through `explorer.exe shell:AppsFolder\…`, as the
//! Start menu does, and shows the icon Explorer shows for it, read by the
//! icon helper process and kept as a PNG beside the entries. Entries Lulo
//! wrote earlier for apps no longer there are removed; nothing else in the
//! folder is touched.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Marks the entries this module owns.
const MARKER: &str = "X-Lulo-Windows-App=true";

/// One app in the Apps folder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct App {
    pub name: String,
    /// Its parsing name below `shell:AppsFolder`.
    pub parsing: String,
}

/// The id the window list gives `app`'s windows, if it can tell: a Store
/// app's AUMID (`Package!App`), or a desktop app's executable file name.
/// Shortcuts to documents, folders and websites have none.
pub fn window_app_id(app: &App) -> Option<String> {
    let parsing = app.parsing.trim();
    if parsing.contains('!') && !parsing.contains('\\') {
        return Some(parsing.to_owned());
    }
    let file = parsing.rsplit(['\\', '/']).next()?;
    file.to_ascii_lowercase()
        .ends_with(".exe")
        .then(|| file.to_ascii_lowercase())
}

/// Whether `app` is one of Lulo's own (they have their own entries).
fn is_lulo(id: &str) -> bool {
    rmac_apps::windows_apps::app_for_exe(id).is_some()
}

/// A desktop entry's string value: no line breaks.
fn value(text: &str) -> String {
    text.replace(['\r', '\n'], " ").trim().to_owned()
}

/// The entry for `app`, with `icon` (an absolute path) if it has one.
pub fn entry_with_icon(app: &App, icon: Option<&Path>) -> String {
    let mut entry = entry(app);
    if let Some(icon) = icon {
        entry.push_str(&format!("Icon={}\n", value(&icon.to_string_lossy())));
    }
    entry
}

/// The entry for `app`.
pub fn entry(app: &App) -> String {
    // `Exec` quoting: a backslash escapes the next character.
    let target = format!("shell:AppsFolder\\{}", app.parsing).replace('\\', "\\\\");
    let target = target.replace('"', "");
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec=explorer.exe \"{}\"\n{MARKER}\n",
        value(&app.name),
        target
    )
}

/// The apps that get entries, by entry file name. When two apps share an
/// id (two shortcuts to one program), the first keeps it.
pub fn chosen(apps: &[App]) -> BTreeMap<String, App> {
    let mut chosen = BTreeMap::new();
    for app in apps {
        let Some(id) = window_app_id(app) else {
            continue;
        };
        if is_lulo(&id) || app.name.to_lowercase().contains("uninstall") {
            continue;
        }
        let file = format!("{id}.desktop");
        if file.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
            continue;
        }
        chosen.entry(file).or_insert_with(|| app.clone());
    }
    chosen
}

/// The entries for `apps`, by file name, without icons.
pub fn entries(apps: &[App]) -> BTreeMap<String, String> {
    chosen(apps)
        .into_iter()
        .map(|(file, app)| (file, entry(&app)))
        .collect()
}

/// Where the icon of the entry `file` is kept.
fn icon_path(icons: &Path, file: &str) -> PathBuf {
    let stem = file.trim_end_matches(".desktop");
    let safe: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    icons.join(format!("{safe}.png"))
}

/// Read the icons `chosen` lacks through the icon helper and keep them as
/// PNGs in `icons`. Blocking.
fn fetch_icons(icons: &Path, chosen: &BTreeMap<String, App>) {
    let missing: BTreeMap<String, PathBuf> = chosen
        .iter()
        .filter(|(file, _)| !icon_path(icons, file).is_file())
        .map(|(file, app)| (format!("shell:AppsFolder\\{}", app.parsing), icon_path(icons, file)))
        .collect();
    if missing.is_empty() || std::fs::create_dir_all(icons).is_err() {
        return;
    }
    let (replies, received) = async_channel::unbounded();
    super::icons::start(replies);
    for source in missing.keys() {
        super::icons::request(source);
    }
    let mut left = missing.len();
    while left > 0 {
        let Ok((source, image)) = received.recv_blocking() else {
            break;
        };
        let Some(path) = missing.get(&source) else {
            continue;
        };
        left -= 1;
        let Some(image) = image else {
            continue;
        };
        let size = image.size(0);
        let Some(bytes) = image.as_bytes(0) else {
            continue;
        };
        // The helper's pixels are BGRA; PNG wants RGBA.
        let mut rgba = bytes.to_vec();
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        if let Some(buffer) =
            image::RgbaImage::from_raw(size.width.0 as u32, size.height.0 as u32, rgba)
        {
            let _ = buffer.save_with_format(path, image::ImageFormat::Png);
        }
    }
}

/// Write `entries` into `applications`, rewriting only those that changed,
/// and remove this module's earlier entries that are gone.
pub fn sync(applications: &Path, entries: &BTreeMap<String, String>) -> std::io::Result<usize> {
    std::fs::create_dir_all(applications)?;
    let mut written = 0;
    for (file, contents) in entries {
        let path = applications.join(file);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(contents.as_str()) {
            std::fs::write(&path, contents)?;
            written += 1;
        }
    }
    for existing in std::fs::read_dir(applications)?.flatten() {
        let name = existing.file_name().to_string_lossy().into_owned();
        if entries.contains_key(&name) || !name.ends_with(".desktop") {
            continue;
        }
        let ours = std::fs::read_to_string(existing.path())
            .is_ok_and(|contents| contents.lines().any(|line| line.trim() == MARKER));
        if ours {
            let _ = std::fs::remove_file(existing.path());
        }
    }
    Ok(written)
}

/// Lulo's applications folder.
fn applications() -> Option<PathBuf> {
    rmac_apps::windows_apps::data_home().map(|home| home.join("applications"))
}

/// Read the Apps folder (through the helper) and bring the entries up to
/// date. Blocking; call it off the UI thread.
pub fn refresh() {
    let Some(applications) = applications() else {
        return;
    };
    let Some(apps) = super::catalog::apps_folder_apps() else {
        super::trace(|| "windows apps: the Apps folder could not be read".into());
        return;
    };
    let chosen = chosen(&apps);
    let icons = applications.join("lulo-windows-icons");
    fetch_icons(&icons, &chosen);
    let entries: BTreeMap<String, String> = chosen
        .iter()
        .map(|(file, app)| {
            let icon = icon_path(&icons, file);
            (file.clone(), entry_with_icon(app, icon.is_file().then_some(icon.as_path())))
        })
        .collect();
    match sync(&applications, &entries) {
        Ok(written) => super::trace(|| {
            format!(
                "windows apps: {} entries ({written} written) from {} Apps folder items",
                entries.len(),
                apps.len()
            )
        }),
        Err(error) => super::trace(|| format!("windows apps: not written: {error}")),
    }
}

/// [`refresh`] now and whenever the Start menu folders change, on a thread
/// of its own.
pub fn start() {
    let spawned = std::thread::Builder::new()
        .name("lulo-windows-apps".into())
        .spawn(|| {
            refresh();
            let (changed_tx, changed_rx) = std::sync::mpsc::channel::<()>();
            super::catalog::watch(move |list| {
                if list == super::catalog::List::Apps {
                    let _ = changed_tx.send(());
                }
            });
            while changed_rx.recv().is_ok() {
                // A burst of changes (an installer) reads the folder once.
                while changed_rx
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .is_ok()
                {}
                refresh();
            }
        });
    if let Err(error) = spawned {
        eprintln!("lulo-shell: Windows apps will not be in Spotlight: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, parsing: &str) -> App {
        App {
            name: name.into(),
            parsing: parsing.into(),
        }
    }

    #[test]
    fn apps_are_named_by_the_id_their_windows_get() {
        assert_eq!(
            window_app_id(&app(
                "Calculator",
                "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"
            ))
            .as_deref(),
            Some("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App")
        );
        assert_eq!(
            window_app_id(&app(
                "Notepad",
                "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\Notepad.exe"
            ))
            .as_deref(),
            Some("notepad.exe")
        );
        assert_eq!(
            window_app_id(&app("Help", "https://example.com/help")),
            None
        );
    }

    #[test]
    fn entries_open_the_app_through_the_apps_folder() {
        let entry = entry(&app("Notepad", "{1AC14E77}\\Notepad.exe"));
        assert!(entry.contains("Name=Notepad\n"));
        assert!(
            entry.contains("Exec=explorer.exe \"shell:AppsFolder\\\\{1AC14E77}\\\\Notepad.exe\"\n")
        );
        assert!(entry.contains(MARKER));
    }

    #[test]
    fn lulo_apps_and_uninstallers_get_no_entry() {
        let entries = entries(&[
            app("Files", "C:\\Lulo\\rmac-files.exe"),
            app("Uninstall Thing", "C:\\Thing\\unins000.exe"),
            app("Paint", "Microsoft.Paint_8wekyb3d8bbwe!App"),
        ]);
        assert_eq!(
            entries.keys().collect::<Vec<_>>(),
            ["Microsoft.Paint_8wekyb3d8bbwe!App.desktop"]
        );
    }

    #[test]
    fn sync_rewrites_changes_and_removes_only_its_own_gone_entries() {
        let folder = std::env::temp_dir().join(format!("lulo-app-entries-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("org.rmac.Notes.desktop"), "[Desktop Entry]\n").unwrap();
        std::fs::write(
            folder.join("old.exe.desktop"),
            format!("[Desktop Entry]\n{MARKER}\n"),
        )
        .unwrap();
        let mut wanted = BTreeMap::new();
        wanted.insert(
            "paint.exe.desktop".to_owned(),
            entry(&app("Paint", "C:\\paint.exe")),
        );
        assert_eq!(sync(&folder, &wanted).unwrap(), 1);
        assert_eq!(sync(&folder, &wanted).unwrap(), 0);
        assert!(folder.join("org.rmac.Notes.desktop").exists());
        assert!(!folder.join("old.exe.desktop").exists());
        let _ = std::fs::remove_dir_all(&folder);
    }
}
