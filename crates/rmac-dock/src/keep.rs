//! Keeping an application in the Dock by dragging it there, as on macOS 26:
//! a running app dragged left across the separator, or an app dragged in
//! from Apps or Files. Geometry-free decisions only: the renderer supplies
//! the resting centres of the kept tiles it drew.
//!
//! Measured on macOS 26.2 (docs/dock-behaviour-2026-09-23.md): while an app
//! is dragged over the kept apps, the kept apps part to open a slot at the
//! pointer; releasing keeps the app in that slot. Over the Trash, or more
//! than 1.5 tiles away from the shelf, no slot opens and a release keeps
//! nothing.

use std::path::Path;

use crate::reorder::{DRAG_THRESHOLD, REMOVE_DISTANCE_TILES};

/// The kept-app slot a pointer at `axis` opens: the number of kept tiles
/// whose resting centre lies before it.
pub fn insertion_index(centers: &[f32], axis: f32) -> usize {
    centers.iter().filter(|center| **center < axis).count()
}

/// Where an app dragged to `axis` would be kept, or `None` when no slot
/// opens: the pointer is at or past `limit` along the Dock (the separator
/// before the running apps, or the one before the Trash), or `lift` (the
/// pointer's distance beyond the shelf's inner edge) is at the distance
/// where a kept app's drag shows "Remove".
pub fn keep_destination(
    centers: &[f32],
    limit: f32,
    axis: f32,
    lift: f32,
    tile: f32,
) -> Option<usize> {
    let usable =
        axis.is_finite() && lift.is_finite() && limit.is_finite() && tile.is_finite() && tile > 0.0;
    (usable && axis < limit && lift < REMOVE_DISTANCE_TILES * tile)
        .then(|| insertion_index(centers, axis))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeepDrop {
    /// Never crossed the threshold: an ordinary click.
    Click,
    /// Released where no slot was open; the app stays where it was.
    NoChange,
    /// Released over the kept apps: keep the app at this index.
    Keep { at: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeepDragState {
    pub active: bool,
    /// The open kept-app slot, if the pointer is over the kept apps.
    pub destination: Option<usize>,
    /// Something the renderer shows changed since the previous update.
    pub changed: bool,
}

/// A press on a running app that is not kept, which becomes a drag once
/// the pointer moves and keeps the app if it is released over the kept
/// apps.
#[derive(Clone, Debug, PartialEq)]
pub struct KeepDrag {
    centers: Vec<f32>,
    limit: f32,
    press_axis: f32,
    press_lift: f32,
    tile: f32,
    active: bool,
    destination: Option<usize>,
}

impl KeepDrag {
    /// `centers` are the kept tiles' resting centres (ascending, possibly
    /// none); `limit` is the separator between them and the running apps.
    pub fn begin(
        centers: Vec<f32>,
        limit: f32,
        press_axis: f32,
        press_lift: f32,
        tile: f32,
    ) -> Option<Self> {
        let usable = limit.is_finite()
            && press_axis.is_finite()
            && press_lift.is_finite()
            && tile.is_finite()
            && tile > 0.0
            && centers
                .iter()
                .all(|center| center.is_finite() && *center < limit)
            && centers.windows(2).all(|pair| pair[0] < pair[1]);
        usable.then_some(Self {
            centers,
            limit,
            press_axis,
            press_lift,
            tile,
            active: false,
            destination: None,
        })
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn state(&self) -> KeepDragState {
        KeepDragState {
            active: self.active,
            destination: self.destination,
            changed: false,
        }
    }

    pub fn update(&mut self, axis: f32, lift: f32) -> KeepDragState {
        if !axis.is_finite() || !lift.is_finite() {
            return self.state();
        }
        let before = (self.active, self.destination);
        if !self.active
            && ((axis - self.press_axis).abs() >= DRAG_THRESHOLD
                || (lift - self.press_lift).abs() >= DRAG_THRESHOLD)
        {
            self.active = true;
        }
        if self.active {
            self.destination = keep_destination(&self.centers, self.limit, axis, lift, self.tile);
        }
        let mut state = self.state();
        state.changed = before != (self.active, self.destination);
        state
    }

    pub fn finish(&self) -> KeepDrop {
        match (self.active, self.destination) {
            (false, _) => KeepDrop::Click,
            (true, None) => KeepDrop::NoChange,
            (true, Some(at)) => KeepDrop::Keep { at },
        }
    }
}

/// Whether a dropped file is an application the Dock can keep: a desktop
/// entry, or on Windows a shortcut or a program.
pub fn is_application_file(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };
    extension.eq_ignore_ascii_case("desktop")
        || (cfg!(windows)
            && (extension.eq_ignore_ascii_case("lnk") || extension.eq_ignore_ascii_case("exe")))
}

/// Whether every dropped path is an application, so the drop keeps apps
/// rather than opening or trashing files.
pub fn all_applications(paths: &[std::path::PathBuf]) -> bool {
    !paths.is_empty() && paths.iter().all(|path| is_application_file(path))
}

/// The catalog application a dropped file names: a desktop entry by its
/// path or file name; a program by its file name (Lulo's own programs by
/// their identity); a shortcut by its name, as the Start menu names apps.
pub fn resolve_dropped_application<'a>(
    catalog: &'a [rmac_apps::Application],
    path: &Path,
) -> Option<&'a rmac_apps::Application> {
    let file_name = path.file_name()?.to_str()?;
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "desktop" => catalog
            .iter()
            .find(|application| application.source == path || application.id == file_name),
        "exe" => {
            let own = rmac_apps::windows_apps::app_for_exe(file_name)
                .map(|app| crate::dock::canonical_app_id(app.app_id));
            let program = crate::dock::canonical_app_id(file_name);
            catalog.iter().find(|application| {
                let id = crate::dock::canonical_app_id(&application.id);
                own.as_deref() == Some(id.as_str()) || id == program
            })
        }
        "lnk" => {
            let stem = path.file_stem()?.to_str()?.trim();
            catalog
                .iter()
                .find(|application| application.name.trim().eq_ignore_ascii_case(stem))
        }
        _ => None,
    }
}

/// A desktop entry for a program dropped on the Dock that no installed app
/// names, so it can be kept (Windows): `(file name, contents)`. The file
/// name is the window app id the Dock gives the program's windows (its
/// executable's file name), so its running dot lands on the kept tile.
pub fn program_entry(program: &Path) -> Option<(String, String)> {
    let file_name = program.file_name()?.to_str()?;
    if !file_name.to_ascii_lowercase().ends_with(".exe") || !program.is_absolute() {
        return None;
    }
    let text = program.to_str()?;
    if text.contains(['"', '\r', '\n']) {
        return None;
    }
    let name = rmac_apps::windows_apps::name_from_exe(file_name);
    // `Exec` quoting: a backslash escapes the next character.
    let exec = text.replace('\\', "\\\\");
    Some((
        format!("{}.desktop", file_name.to_ascii_lowercase()),
        format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec=\"{exec}\"\nX-Lulo-Kept-Program=true\n"
        ),
    ))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const TILE: f32 = 64.0;

    fn centers() -> Vec<f32> {
        // Four kept apps at the measured 68 pt pitch.
        (0..4).map(|index| 100.0 + 68.0 * index as f32).collect()
    }

    const SEPARATOR: f32 = 100.0 + 68.0 * 3.0 + 50.0;

    #[test]
    fn the_slot_opens_before_the_first_centre_the_pointer_has_not_passed() {
        let centers = centers();
        assert_eq!(insertion_index(&centers, 0.0), 0);
        assert_eq!(insertion_index(&centers, 99.0), 0);
        assert_eq!(insertion_index(&centers, 101.0), 1);
        assert_eq!(insertion_index(&centers, 200.0), 2);
        assert_eq!(insertion_index(&centers, 900.0), 4);
        assert_eq!(insertion_index(&[], 50.0), 0);
    }

    #[test]
    fn no_slot_opens_past_the_limit_or_far_above_the_shelf() {
        let centers = centers();
        assert_eq!(
            keep_destination(&centers, SEPARATOR, 150.0, -20.0, TILE),
            Some(1)
        );
        // Over the running apps (or the Trash): nothing opens.
        assert_eq!(
            keep_destination(&centers, SEPARATOR, SEPARATOR, -20.0, TILE),
            None
        );
        // At the "Remove" distance above the shelf: nothing opens.
        assert_eq!(
            keep_destination(&centers, SEPARATOR, 150.0, 96.0, TILE),
            None
        );
        assert_eq!(
            keep_destination(&centers, SEPARATOR, 150.0, 95.0, TILE),
            Some(1)
        );
        assert_eq!(
            keep_destination(&centers, SEPARATOR, f32::NAN, 0.0, TILE),
            None
        );
    }

    #[test]
    fn a_running_app_is_kept_where_it_crosses_into_the_kept_apps() {
        let press = SEPARATOR + 40.0;
        let mut drag = KeepDrag::begin(centers(), SEPARATOR, press, -30.0, TILE).unwrap();
        // Below the threshold: a click.
        assert!(!drag.update(press - 2.0, -30.0).active);
        assert_eq!(drag.finish(), KeepDrop::Click);
        // Moving within the running apps opens nothing.
        let state = drag.update(press + 30.0, -30.0);
        assert!(state.active && state.changed);
        assert_eq!(state.destination, None);
        assert_eq!(drag.finish(), KeepDrop::NoChange);
        // Across the separator: the slot opens after the last kept app.
        let state = drag.update(SEPARATOR - 10.0, -30.0);
        assert_eq!(state.destination, Some(4));
        assert!(state.changed);
        // Further left: between the first and second kept apps.
        assert_eq!(drag.update(150.0, -30.0).destination, Some(1));
        assert!(!drag.update(151.0, -30.0).changed);
        assert_eq!(drag.finish(), KeepDrop::Keep { at: 1 });
        // Pulled off the Dock: the slot closes and a release keeps nothing.
        assert_eq!(drag.update(150.0, 200.0).destination, None);
        assert_eq!(drag.finish(), KeepDrop::NoChange);
    }

    #[test]
    fn a_running_app_with_no_kept_apps_opens_the_first_slot() {
        let mut drag = KeepDrag::begin(Vec::new(), 90.0, 120.0, -30.0, TILE).unwrap();
        drag.update(60.0, -30.0);
        assert_eq!(drag.finish(), KeepDrop::Keep { at: 0 });
    }

    #[test]
    fn malformed_keep_drags_are_rejected() {
        assert!(KeepDrag::begin(centers(), 120.0, 0.0, 0.0, TILE).is_none());
        assert!(KeepDrag::begin(vec![20.0, 10.0], SEPARATOR, 0.0, 0.0, TILE).is_none());
        assert!(KeepDrag::begin(centers(), f32::NAN, 0.0, 0.0, TILE).is_none());
        assert!(KeepDrag::begin(centers(), SEPARATOR, 0.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn application_files_are_recognised() {
        assert!(is_application_file(Path::new(
            "/usr/share/applications/org.rmac.Notes.desktop"
        )));
        assert!(is_application_file(Path::new("/a/B.DESKTOP")));
        assert!(!is_application_file(Path::new("/a/notes.txt")));
        assert!(!is_application_file(Path::new("/a/folder")));
        assert_eq!(
            is_application_file(Path::new("C:/Apps/Paint.lnk")),
            cfg!(windows)
        );
        assert!(all_applications(&[PathBuf::from("/a/x.desktop")]));
        assert!(!all_applications(&[
            PathBuf::from("/a/x.desktop"),
            PathBuf::from("/a/y.txt")
        ]));
        assert!(!all_applications(&[]));
    }

    fn application(id: &str, name: &str, source: &str) -> rmac_apps::Application {
        rmac_apps::Application {
            id: id.into(),
            name: name.into(),
            generic_name: None,
            keywords: Vec::new(),
            source: PathBuf::from(source),
            icon: None,
            categories: Vec::new(),
            mime_types: Vec::new(),
            launch: rmac_apps::LaunchSpec::Command {
                program: id.into(),
                args: Vec::new(),
                working_dir: None,
                terminal: false,
            },
            actions: Vec::new(),
        }
    }

    #[test]
    fn dropped_files_resolve_to_catalog_applications() {
        let catalog = [
            application(
                "org.rmac.Notes.desktop",
                "Notes",
                "/usr/share/applications/org.rmac.Notes.desktop",
            ),
            application(
                "notepad.exe.desktop",
                "Notepad",
                "/data/notepad.exe.desktop",
            ),
        ];
        let id = |path: &str| {
            resolve_dropped_application(&catalog, Path::new(path)).map(|app| app.id.as_str())
        };
        assert_eq!(
            id("/usr/share/applications/org.rmac.Notes.desktop"),
            Some("org.rmac.Notes.desktop")
        );
        // A copy elsewhere still names the installed entry by file name.
        assert_eq!(
            id("/home/me/Desktop/org.rmac.Notes.desktop"),
            Some("org.rmac.Notes.desktop")
        );
        assert_eq!(id("C:/Lulo/rmac-notes.exe"), Some("org.rmac.Notes.desktop"));
        assert_eq!(id("C:/Windows/NOTEPAD.EXE"), Some("notepad.exe.desktop"));
        assert_eq!(id("C:/Start Menu/Notepad.lnk"), Some("notepad.exe.desktop"));
        assert_eq!(id("/a/unknown.desktop"), None);
        assert_eq!(id("/a/notes.txt"), None);
    }

    #[test]
    fn an_unknown_program_gets_its_own_entry() {
        let (program, exec) = if cfg!(windows) {
            (r"C:\Tools\Paint.NET.exe", r"C:\\Tools\\Paint.NET.exe")
        } else {
            ("/opt/Tools/Paint.NET.exe", "/opt/Tools/Paint.NET.exe")
        };
        let (file, entry) = program_entry(Path::new(program)).unwrap();
        assert_eq!(file, "paint.net.exe.desktop");
        assert!(entry.contains(&format!("Exec=\"{exec}\"\n")));
        assert!(entry.contains("Name=Paint.NET\n"));
        assert!(entry.contains("X-Lulo-Kept-Program=true"));
        assert!(entry.starts_with("[Desktop Entry]\nType=Application\nName="));
        assert!(program_entry(Path::new("relative/tool.exe")).is_none());
        assert!(program_entry(Path::new("/opt/tool.txt")).is_none());
    }
}
